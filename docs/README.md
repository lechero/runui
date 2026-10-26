# runui guide

Open [index.html](index.html) for the illustrated app guide, including the [run and output tour](index.html#workflow), [tmux status bars](index.html#status-bars), and [MCP agent tools](index.html#mcp). The [brand kit](brand.html) has the logo, icons, colors, and download links. Both pages are static HTML with no build step, JavaScript, external fonts, or network dependency.

The media show Runui running against a disposable sample project. To regenerate them:

```sh
task build
python3 -m pip install Pillow
python3 scripts/capture-docs.py
```

The capture script also needs `tmux`. It creates a temporary home, project files, and a private tmux server, then stops its demo run and removes the temporary files. It never uses your runui favorites or run history.

`assets/runui-mark.svg` and `assets/runui-logo.svg` are the source brand artwork. To regenerate favicon PNGs, the touch/app icons, and the social card from them, install `rsvg-convert` (librsvg) and run:

```sh
python3 scripts/build-brand-assets.py
```

## Command discovery roadmap

The [command discovery research](command-discovery-research.md) reviews the current implementation, local project inventory, reproduced gaps, and relevant tools. The [integration plan](command-discovery-plan.md) proposes the catalog, palette, workspace support, additional sources, and execution integrations in staged milestones.
