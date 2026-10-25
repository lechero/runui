# runui guide

Open [index.html](index.html) for the app guide, including [MCP agent tools](index.html#mcp), and [brand.html](brand.html) for the logo, icons, colors, and download links. Both pages are static HTML with no build step, JavaScript, external fonts, or network dependency.

The media show the real Rust binary running against a disposable sample project. To regenerate them:

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
