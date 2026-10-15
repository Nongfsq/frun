# frun

[中文](README.zh-CN.md)

A program launcher for Windows. Press Alt + Space, type a few letters, hit Enter, and the program opens.

It looks for programs, never files. It finds them wherever they live: the Start menu, the Microsoft Store, and the little tools you keep in folders like D:\Tools that never made it into the Start menu.

## Using it

- **Alt + Space** to show it; press again or Esc to hide.
- Type a name, or the first letters of its pinyin: `wx` finds 微信 (WeChat).
- **Enter** opens; **Alt + a number** opens that row.
- One program, one row. Press **→** to see the other exes in its folder.
- Type a path like `d:\`, **Tab** to complete, Enter to open the folder.
- Right-click or shortcuts: open as administrator (Ctrl + Shift + Enter), open the containing folder (Ctrl + Enter), hide it (Ctrl + Del, Ctrl + Z to undo).
- **Ctrl + ,** opens settings.

There is nothing to set up on first run. frun reads the install records Windows keeps, works out which folders hold your programs, and adds them. New programs show up on their own.

## Light

One exe of about 2 MB, no runtime, no web view. On the author's machine, a search takes about 0.25 ms per keystroke, and three seconds after it hides it holds about 0.4 MB of physical memory.

## Building

Windows 10 or 11 and Rust (2024 edition).

```
cargo build --release
```

The program ends up at `target/release/frun.exe`.

## License

MIT, see [LICENSE](LICENSE). The interface font is a subset of Sarasa Gothic under the SIL Open Font License 1.1, see [assets/FONT-NOTICE.md](assets/FONT-NOTICE.md).
