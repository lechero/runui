//! What a Maven pom builds, and the profiles it and its modules have.

use std::collections::{HashMap, HashSet};

use runui_core::golang::filepath;

use crate::xml::{self, Token};
use crate::{Command, MAX_DEPTH, Source, is_file, quote};

/// aem_profile says what the AEM project archetype's profiles do, which
/// their pom doesn't.
fn aem_profile(id: &str) -> Option<&'static str> {
    Some(match id {
        "autoInstallSinglePackage" => {
            "Build it all and deploy it as one package to AEM's author (localhost:4502 unless aem.host and aem.port say otherwise)"
        }
        "autoInstallSinglePackagePublish" => {
            "Build it all and deploy it as one package to AEM's publish (localhost:4503 unless aem.publish.host and aem.publish.port say otherwise)"
        }
        "autoInstallPackage" => "Build and deploy each content package to AEM's author",
        "autoInstallPackagePublish" => "Build and deploy each content package to AEM's publish",
        "autoInstallBundle" => "Build and deploy the OSGi bundle to AEM's author",
        _ => return None,
    })
}

/// read_pom is what a Maven pom builds: clean install, clean verify and
/// test, then clean install with each profile that isn't on by default:
/// its own, then the ones its modules have (the AEM archetype's deploy
/// profiles are in core, ui.apps and ui.content), which the build from
/// here turns on in them. It runs through the project's Maven wrapper
/// where it has one (mvnw in the pom's folder or above, up to stop).
pub fn read_pom(path: &str, stop: &str) -> (Vec<Command>, Option<anyhow::Error>) {
    read_tracked(path, stop, &mut crate::discovery::Scan::new(&filepath::dir(path)))
}

pub(crate) fn read_tracked(
    path: &str,
    stop: &str,
    scan: &mut crate::discovery::Scan,
) -> (Vec<Command>, Option<anyhow::Error>) {
    let pom = match read_pom_file(path, scan) {
        Ok(pom) => pom,
        Err(e) => return (Vec::new(), Some(e)),
    };
    let dir = filepath::dir(path);
    let mvn = maven_command(&dir, stop, scan);
    let what = if pom.modules.is_empty() { "it" } else { "every module" };
    let cmd = |goals: String, arguments: Vec<String>, desc: String| {
        let argv: Vec<_> = std::iter::once(mvn.clone()).chain(arguments).collect();
        Command {
            source: Source::Maven,
            run: argv.iter().map(|argument| quote(argument)).collect::<Vec<_>>().join(" "),
            argv,
            args_separator: false,
            defining_file: path.into(),
            defining_line: None,
            variant: String::new(),
            generated: true,
            name: goals,
            desc,
        }
    };
    let mut cmds = vec![
        cmd(
            "clean install".into(),
            vec!["clean".into(), "install".into()],
            format!("Build {what} and install it in ~/.m2"),
        ),
        cmd(
            "clean verify".into(),
            vec!["clean".into(), "verify".into()],
            format!("Build {what} and run the tests, installing nothing"),
        ),
        cmd("test".into(), vec!["test".into()], "Run the unit tests".into()),
    ];
    let mut ids: Vec<String> = Vec::new();
    // the modules a profile is in, when it's theirs
    let mut within: HashMap<String, Vec<String>> = HashMap::new();
    let mut own: HashSet<String> = HashSet::new();
    for id in pom.profiles() {
        ids.push(id.clone());
        own.insert(id);
    }
    for (module, profiles) in modules_profiles(&dir, &pom.modules, 0, scan) {
        for id in profiles {
            if own.contains(&id) {
                continue;
            }
            if !within.contains_key(&id) {
                ids.push(id.clone());
            }
            within.entry(id).or_default().push(module.clone());
        }
    }
    for id in ids {
        let mut desc = aem_profile(&id).map_or_else(|| format!("clean install with the {id} profile"), str::to_string);
        if let Some(modules) = within.get(&id)
            && !modules.is_empty()
        {
            desc += &format!(" ({})", modules.join(", "));
        }
        cmds.push(cmd(
            format!("clean install -P{}", quote(&id)),
            vec!["clean".into(), "install".into(), format!("-P{id}")],
            desc,
        ));
    }
    (cmds, None)
}

/// Pom is what runui reads of a pom.xml.
#[derive(Debug, Default)]
struct Pom {
    modules: Vec<String>,
    profiles: Vec<Profile>,
}

#[derive(Debug, Default)]
struct Profile {
    id: String,
    active_by_default: String,
}

impl Pom {
    /// profiles are the pom's profiles that aren't on by default: one that
    /// is is in every build already.
    fn profiles(&self) -> Vec<String> {
        self.profiles
            .iter()
            .filter(|pr| !pr.id.trim().is_empty() && pr.active_by_default.trim() != "true")
            .map(|pr| pr.id.trim().to_string())
            .collect()
    }
}

fn read_pom_file(path: &str, scan: &mut crate::discovery::Scan) -> anyhow::Result<Pom> {
    let data = scan.read(path)?;
    parse_pom(&data).map_err(|e| anyhow::anyhow!("pom.xml: {e}"))
}

/// parse_pom decodes a pom as Go's xml.Unmarshal decodes it into
/// `struct { XMLName xml.Name "xml:\"project\""; Modules []string
/// "xml:\"modules>module\""; Profiles []struct{…} "xml:\"profiles>profile\"" }`:
/// the document's first element, which must be a project, its modules'
/// module elements' text, and its profiles' profile elements, each one's id
/// and activation's activeByDefault; every other element skipped.
fn parse_pom(data: &[u8]) -> Result<Pom, xml::Error> {
    let mut d = xml::Decoder::new(data);
    let start = loop {
        if let Token::StartElement(t) = d.token()? {
            break t;
        }
    };
    if start.name.local != "project" {
        return Err(xml::Error::Other(format!("expected element type <project> but have <{}>", start.name.local)));
    }
    let mut p = Pom::default();
    loop {
        match d.token()? {
            Token::StartElement(t) if t.name.local == "modules" => each(&mut d, "module", &mut |d| {
                p.modules.push(text(d)?);
                Ok(())
            })?,
            Token::StartElement(t) if t.name.local == "profiles" => each(&mut d, "profile", &mut |d| {
                p.profiles.push(profile(d)?);
                Ok(())
            })?,
            Token::StartElement(_) => d.skip()?,
            Token::EndElement(_) => return Ok(p),
            _ => {}
        }
    }
}

/// each decodes the elements named so in the one being read (a path's
/// last step, modules>module), skipping the rest, up to its end.
fn each(
    d: &mut xml::Decoder,
    name: &str,
    decode: &mut dyn FnMut(&mut xml::Decoder) -> Result<(), xml::Error>,
) -> Result<(), xml::Error> {
    loop {
        match d.token()? {
            Token::StartElement(t) if t.name.local == name => decode(d)?,
            Token::StartElement(_) => d.skip()?,
            Token::EndElement(_) => return Ok(()),
            _ => {}
        }
    }
}

/// text is an element decoded into a string: the text right in it, the
/// elements in it skipped.
fn text(d: &mut xml::Decoder) -> Result<String, xml::Error> {
    let mut s = String::new();
    loop {
        match d.token()? {
            Token::StartElement(_) => d.skip()?,
            Token::EndElement(_) => return Ok(s),
            Token::CharData(c) => s.push_str(&c),
            _ => {}
        }
    }
}

/// profile is a profile element: its id, and its activation's
/// activeByDefault, the last of each.
fn profile(d: &mut xml::Decoder) -> Result<Profile, xml::Error> {
    let mut pr = Profile::default();
    loop {
        match d.token()? {
            Token::StartElement(t) if t.name.local == "id" => pr.id = text(d)?,
            Token::StartElement(t) if t.name.local == "activation" => loop {
                match d.token()? {
                    Token::StartElement(t) if t.name.local == "activeByDefault" => pr.active_by_default = text(d)?,
                    Token::StartElement(_) => d.skip()?,
                    Token::EndElement(_) => break,
                    _ => {}
                }
            },
            Token::StartElement(_) => d.skip()?,
            Token::EndElement(_) => return Ok(pr),
            _ => {}
        }
    }
}

/// modules_profiles walks a pom's modules, and theirs, a few deep, in the
/// order they're listed, saying each one's profiles; a module whose pom
/// can't be read is left out.
fn modules_profiles(
    dir: &str,
    modules: &[String],
    depth: usize,
    scan: &mut crate::discovery::Scan,
) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    if depth >= MAX_DEPTH {
        if !modules.is_empty() {
            scan.problem(Some(Source::Maven), dir, "module discovery reached its depth limit");
        }
        return out;
    }
    for m in modules {
        if !scan.expand(Source::Maven, dir) {
            break;
        }
        let mdir = filepath::join(&[dir, m]);
        let path = filepath::join(&[&mdir, "pom.xml"]);
        let p = match read_pom_file(&path, scan) {
            Ok(pom) => pom,
            Err(err) => {
                scan.problem(Some(Source::Maven), &path, err.to_string());
                continue;
            }
        };
        out.push((m.clone(), p.profiles()));
        for (sub, profiles) in modules_profiles(&mdir, &p.modules, depth + 1, scan) {
            out.push((format!("{m}/{sub}"), profiles));
        }
    }
    out
}

/// maven_command is how Maven runs in a folder: the project's wrapper, the
/// nearest mvnw from dir up to stop, as a path from dir; else mvn.
fn maven_command(dir: &str, stop: &str, scan: &mut crate::discovery::Scan) -> String {
    let stop = filepath::clean(stop);
    let mut d = filepath::clean(dir);
    loop {
        let wrapper = filepath::join(&[&d, "mvnw"]);
        scan.watch(&wrapper);
        if is_file(&wrapper)
            && let Some(rel) = filepath::rel(dir, &d)
        {
            return format!("{rel}/mvnw"); // ./mvnw, ../mvnw: sh wants the slash
        }
        let parent = filepath::dir(&d);
        if d == stop || parent == d || !std::path::Path::new(&d).starts_with(&stop) {
            return "mvn".to_string();
        }
        d = parent;
    }
}
