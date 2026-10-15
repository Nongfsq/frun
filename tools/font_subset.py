# 从更纱黑体（Sarasa UI SC Regular）上游 TTF 做 assets/frun-ui.woff2 子集：字符表沿用现有子集，
# 只留横排用的表和 calt、ccmp、kern、locl 四个特性，不带 hinting，保留版权和版本信息。
# 需要 fonttools 和 brotli：pip install fonttools brotli
# 用法：python tools/font_subset.py <旧子集.woff2> <上游 SarasaUiSC-Regular.ttf> <输出.woff2>
import sys
from fontTools import subset
from fontTools.ttLib import TTFont

old, src, out = sys.argv[1:4]
codepoints = sorted(TTFont(old).getBestCmap())
opts = subset.Options()
opts.flavor = 'woff2'
opts.hinting = False
opts.layout_features = ['calt', 'ccmp', 'kern', 'locl']
opts.name_IDs = [0, 1, 2, 3, 4, 5, 6]  # 留着版权、版本和字体名
opts.name_languages = [0x0409]
opts.notdef_outline = True
opts.drop_tables += ['VORG', 'vhea', 'vmtx']  # 只排横排，竖排的度量用不到
font = TTFont(src)
sub = subset.Subsetter(opts)
sub.populate(unicodes=codepoints)
sub.subset(font)
font.flavor = 'woff2'
font.save(out)
new = TTFont(out)
missing = [hex(c) for c in codepoints if c not in new.getBestCmap()]
print('codepoints', len(codepoints), 'missing in new', len(missing), missing[:10])
