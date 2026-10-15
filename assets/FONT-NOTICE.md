# 内嵌字体

`frun-ui.woff2` 是 Sarasa UI SC Regular（更纱黑体）的子集：拉丁字符 + GB2312，约 7,900 个字形。

- 来源：https://github.com/be5invis/sarasa-gothic
- 许可：SIL Open Font License 1.1（https://openfontlicense.org）
- 子集制作：fonttools `pyftsubset`，输出 WOFF2。当前文件由本机安装的 1.0.13 版制作；
  正式发布前应换成上游最新版重新制作。

发布二进制时，OFL 要求随附上游的版权声明与许可全文。子集过程去掉了字体内部的版权字段，
所以发布包里必须带上上游仓库的 `LICENSE` 文件——这一步还没做（见 `docs/progress/`）。
