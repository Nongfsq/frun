<p align="center">
  <img src="docs/media/logo.png" width="112" alt="frun">
</p>

<h1 align="center">frun</h1>

<p align="center">
  <b>Alt + Space，打几个字母，回车。</b><br>
  Windows 上一个小而安静的程序启动器。
</p>

<p align="center">
  <a href="README.md">English</a>
  &nbsp;·&nbsp;
  <a href="#按键">按键</a>
  &nbsp;·&nbsp;
  <a href="#下载">下载</a>
  &nbsp;·&nbsp;
  <a href="LICENSE">MIT 许可</a>
</p>

<p align="center">
  <img src="docs/media/demo.zh-CN.gif" width="92%" alt="打 wx，回车，微信就开了">
</p>

frun 只找程序，别的都不管。装在哪里的程序都能找到：开始菜单里的，Microsoft Store 的，还有你放在 `D:\Tools` 这种文件夹里、从没进过开始菜单的小工具。你的文档不会混进来。

## 能做什么

### 打名字，或者拼音首字母

`wx` 就是微信，`vsc` 就是 Visual Studio Code。回车打开第一行，Alt + 数字打开第几行。

### 一个程序只占一行

一个程序目录里常常有十几个 exe：主程序、更新器、崩溃上报、各种辅助工具。frun 只把程序列一次，其余的收在它后面，按 → 才展开。

<p align="center">
  <img src="docs/media/one-row.zh-CN.jpg" width="80%" alt="VirtualBox 只出一行，按 → 列出同目录的其他程序">
</p>

### 路径也能直接去

打 `d:\so`，Tab 补全，接着打，回车，文件夹就在资源管理器里打开了。

<p align="center">
  <img src="docs/media/path.jpg" width="80%" alt="输入路径，Tab 补全">
</p>

### 其余的在右键里

以管理员身份打开、打开所在文件夹、不再显示某个程序（Ctrl + Z 撤销）、不再查某个文件夹。

<p align="center">
  <img src="docs/media/menu.zh-CN.jpg" width="80%" alt="右键菜单">
</p>

### 深色和浅色

跟着 Windows 走，也可以自己选。有多块屏幕时，它出现在鼠标所在的那一块。

<p align="center">
  <img src="docs/media/light.zh-CN.jpg" width="80%" alt="浅色主题下的设置">
</p>

### 想要的话，一律以管理员身份打开

在 设置 → 通用 里打开"以管理员身份打开"，之后从 frun 启动的程序都以管理员运行。Windows 照样每次弹窗确认，这是应该的。frun 自己从不要管理员权限。Microsoft Store 的应用不受影响，因为 Windows 不允许它们这样提权运行。

<p align="center">
  <img src="docs/media/admin-switch.zh-CN.jpg" width="49%" alt="以管理员身份打开的开关">
  <img src="docs/media/admin-run.zh-CN.jpg" width="49%" alt="从 frun 打开的 Windows PowerShell，以管理员身份运行">
</p>

### 不用设置

第一次运行时，frun 读 Windows 自己的安装记录，找出你的程序装在哪些文件夹，自己加进来。以后装了新程序，也会自己出现。

## 按键

| 按键 | 作用 |
| --- | --- |
| Alt + Space | 呼出或收起（设置里可以改） |
| 回车 | 打开 |
| Alt + 1 … 8 | 打开第几行 |
| → | 展开程序目录里的其他 exe（← 收起）；路径模式下接受补全 |
| Tab | 路径模式下接受补全；平时移到下一行 |
| Ctrl + Shift + 回车 | 以管理员身份打开 |
| Ctrl + 回车 | 打开所在文件夹 |
| Ctrl + Delete | 不再显示这个程序；Ctrl + Z 撤销 |
| Ctrl + , | 设置 |
| Esc | 返回，或收起 |

## 小，是有意的

一个 2 MB 左右的 exe，用 Rust 写成。不用安装，不带运行库，不用网页渲染。以下是在作者电脑上、索引了 1,603 个程序时的实测：

<p align="center">
  <img src="docs/media/stats.zh-CN.png" width="100%" alt="收起后内存 0.4 MB，每次按键检索 0.25 ms，99.4% 的文件夹直接跳过，整个程序 2 MB">
</p>

## 下载

在 [Releases](https://github.com/Nongfsq/frun/releases/latest) 下载 `frun.exe`，放进一个单独的文件夹运行就行，需要 Windows 10 或 11。这个文件夹别放在 AppData 里：清理工具可能把 AppData 清空，"开机自启"就找不到它了。想开机就有，在设置里打开"开机自启"。新版本会自己来：frun 在后台下载，用发布时附上的 SHA-256 核对，下次收起时换上。

exe 目前还没有代码签名，第一次运行时 Windows SmartScreen 可能提示"Windows 已保护你的电脑"。点"更多信息"，再点"仍要运行"。

想自己构建，装好 [Rust](https://rustup.rs) 后运行：

```
cargo build --release
```

不想用了：在设置里关掉"开机自启"，从托盘图标退出，再删掉 `frun.exe` 以及 `%APPDATA%\frun`、`%LOCALAPPDATA%\frun` 两个文件夹。

## 你的数据

设置和使用次数存在 `%APPDATA%\frun`；索引和图标缓存存在 `%LOCALAPPDATA%\frun`，随时可以删。frun 不往外发送任何东西。它唯一的联网，是每天最多一次去本仓库的 Releases 页面看有没有新版本，可以在设置里关掉。

如果 frun 漏了某个程序，或者显示了不该显示的，运行 `frun --doctor`。它会把报告写到 `%LOCALAPPDATA%\frun\doctor.txt` 并打开。报告里有你电脑上的文件夹和程序路径，附到 issue 之前请先看一遍。issue 怎么处理、为什么不接受外部的 pull request，见 [CONTRIBUTING.md](CONTRIBUTING.md)（英文）。

## 许可

MIT，见 [LICENSE](LICENSE)。界面字体是更纱黑体的子集，使用 SIL Open Font License 1.1，见 [assets/FONT-NOTICE.md](assets/FONT-NOTICE.md)。 "小，是有意的"一节图中的 Rust logo 版权归 Rust Foundation，依 [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) 使用，改为白色；frun 与 Rust 项目无隶属关系。
