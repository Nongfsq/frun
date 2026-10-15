<p align="center">
  <img src="docs/media/logo.png" width="112" alt="frun">
</p>

<h1 align="center">frun</h1>

<p align="center">
  <b>Alt + Space. A few letters. Enter.</b><br>
  A small, quiet program launcher for Windows.
</p>

<p align="center">
  <a href="README.zh-CN.md">中文</a>
  &nbsp;·&nbsp;
  <a href="#keys">Keys</a>
  &nbsp;·&nbsp;
  <a href="#build-it">Build it</a>
  &nbsp;·&nbsp;
  <a href="LICENSE">MIT License</a>
</p>

<p align="center">
  <img src="docs/media/demo.en.gif" width="92%" alt="Type we, press Enter, and WeChat opens">
</p>

frun looks for programs and nothing else. It finds them wherever they live: the Start menu, the Microsoft Store, and the small tools you keep in folders like `D:\Tools` that never made it into the Start menu. Your documents stay out of the way.

## What it does

### Type a name, or its pinyin

`wx` finds 微信 (WeChat), `vsc` finds Visual Studio Code. Enter opens the top row; Alt + a number opens that row.

### One program, one row

A program folder often holds a dozen exes: the app, its updater, crash reporters, helpers. frun lists the program once and keeps the rest behind it. Press → to see them.

<p align="center">
  <img src="docs/media/one-row.en.jpg" width="80%" alt="VirtualBox shows as one row; the arrow key lists the other exes in its folder">
</p>

### Paths work too

Type `d:\so`, press Tab to complete, keep typing, press Enter, and the folder opens in Explorer.

<p align="center">
  <img src="docs/media/path.jpg" width="80%" alt="Typing a path with Tab completion">
</p>

### Right-click for the rest

Open as administrator, open the containing folder, hide a program you never want to see again (Ctrl + Z brings it back), or stop searching a folder.

<p align="center">
  <img src="docs/media/menu.en.jpg" width="80%" alt="The right-click menu">
</p>

### Light and dark

It follows the Windows theme, or you can pick one. With more than one screen, it opens on the screen your mouse is on.

<p align="center">
  <img src="docs/media/light.en.jpg" width="80%" alt="Settings in the light theme">
</p>

### Open everything as administrator, if you want

Turn on *Open as administrator* in Settings → General, and every program you start from frun runs elevated. Windows still asks each time, as it should. frun itself never asks for administrator rights. Microsoft Store apps are left alone, because Windows does not let them run elevated this way.

<p align="center">
  <img src="docs/media/admin-switch.en.jpg" width="49%" alt="The Open as administrator switch">
  <img src="docs/media/admin-run.en.jpg" width="49%" alt="Windows PowerShell opened from frun, running as administrator">
</p>

### Nothing to set up

On first run frun reads the install records Windows keeps, works out which folders hold your programs, and adds them. When you install something new, it shows up on its own.

## Keys

| Key | What it does |
| --- | --- |
| Alt + Space | Show or hide (you can change it in Settings) |
| Enter | Open |
| Alt + 1 … 8 | Open that row |
| → | Show the other exes in a program's folder (← folds them back); accept a path completion |
| Tab | Accept a path completion; otherwise move to the next row |
| Ctrl + Shift + Enter | Open as administrator |
| Ctrl + Enter | Open the containing folder |
| Ctrl + Delete | Hide this program; Ctrl + Z to undo |
| Ctrl + , | Settings |
| Esc | Back, or hide |

## Small on purpose

One exe of about 2 MB, written in Rust. No installer, no runtime, no web view. Measured on the author's machine with 1,603 programs indexed:

<p align="center">
  <img src="docs/media/stats.en.png" width="100%" alt="0.4 MB in memory once hidden, 0.25 ms per keystroke, 99.4% of folders skipped, 2 MB for the whole app">
</p>

## Build it

There is no release yet. Until there is, build it yourself: you need Windows 10 or 11 and [Rust](https://rustup.rs).

```
cargo build --release
```

Put `target/release/frun.exe` anywhere you like and run it. Turn on *Launch at startup* in Settings if you want it there after a restart.

## Your data

Settings and usage counts live in `%APPDATA%\frun`; the index and icon cache live in `%LOCALAPPDATA%\frun` and can be deleted at any time. frun sends nothing anywhere. Its only network request is a check of this repository's Releases page, at most once a day, for a newer version. You can turn that off in Settings.

If frun misses a program or shows one it shouldn't, run `frun --doctor`. It writes a report to `%LOCALAPPDATA%\frun\doctor.txt` and opens it. The report lists folder and program paths from your computer, so read it before you attach it to an issue. How issues are handled, and why pull requests from outside are not accepted, is in [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT, see [LICENSE](LICENSE). The interface font is a subset of Sarasa Gothic under the SIL Open Font License 1.1, see [assets/FONT-NOTICE.md](assets/FONT-NOTICE.md). The Rust logo in the figure under *Small on purpose* is © the Rust Foundation, used under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) and recolored white; frun is not affiliated with the Rust project.
