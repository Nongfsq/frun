# frun

[English](README.md)

Windows 上的程序启动器。按 Alt + Space，打几个字母，回车，程序就开了。

它只找程序，不找文件。装在哪里的程序都能找到：开始菜单里的，Microsoft Store 的，还有你放在 D:\Tools 这种文件夹里、从没进过开始菜单的小工具。

## 用法

- **Alt + Space** 呼出，再按一次或 Esc 收起。
- 直接打名字，或者拼音首字母：`wx` 就是微信。
- **回车**打开；**Alt + 数字**打开第几行。
- 一个程序只占一行。想看它目录里的其他 exe，按 **→**。
- 输入 `d:\` 这样的路径，**Tab** 补全，回车直接打开文件夹。
- 右键或快捷键：以管理员身份打开（Ctrl + Shift + 回车）、打开所在文件夹（Ctrl + 回车）、不再显示（Ctrl + Del，Ctrl + Z 撤销）。
- **Ctrl + ,** 打开设置。

第一次运行不用设置。frun 会从系统的安装记录里找出你的程序装在哪些文件夹，自己加进来，之后有新程序也会自己更新。

## 轻

一个 2 MB 左右的 exe，不带运行库，不用网页渲染。在作者的电脑上：每次按键检索约 0.25 毫秒；收起 3 秒后占用的物理内存约 0.4 MB。

## 从源码构建

需要 Windows 10 或 11，和 Rust（2024 edition）。

```
cargo build --release
```

生成的程序在 `target/release/frun.exe`。

## 许可

MIT，见 [LICENSE](LICENSE)。界面字体是更纱黑体的子集，使用 SIL Open Font License 1.1，见 [assets/FONT-NOTICE.md](assets/FONT-NOTICE.md)。
