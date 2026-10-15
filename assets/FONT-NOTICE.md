# 内嵌字体

`frun-ui.woff2` 是 Sarasa UI SC Regular（更纱黑体）1.0.42 的子集：拉丁字符 + GB2312，7,841 个字符、7,943 个字形。

- 来源：https://github.com/be5invis/sarasa-gothic （发布页的 `SarasaUiSC-TTF-1.0.42.7z`）
- 许可：SIL Open Font License 1.1（https://openfontlicense.org）
- 子集制作：`python tools/font_subset.py <旧子集.woff2> <上游 SarasaUiSC-Regular.ttf> <输出.woff2>`。
  字符表沿用旧子集，只留横排用的表和 calt、ccmp、kern、locl 四个特性，不带 hinting，输出 WOFF2。
  字体内部保留上游的版权、版本和字体名。

发布二进制时，OFL 要求随附上游的版权声明与许可全文。上游仓库的 `LICENSE` 原文放在
`assets/FONT-LICENSE.txt`，每个 Release 都把它作为 `FONT-LICENSE.txt` 和 `frun.exe` 一起发布。

从 1.0.13 换到 1.0.42（2026-10-15）：中文字形渲染出来逐像素相同；拉丁字母里数字 2–9、
`. , : ; < = > ~` 和 `i j m` 的轮廓有细调，逗号和分号窄了约 7%，界面上看不出差别。
