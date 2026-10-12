# 生成 assets/frun.ico：深色圆角方块上一个白色的 f 和一根强调色的输入光标。纯标准库，按有向距离场抗锯齿。
import math, struct, zlib, sys

BG = (0x20, 0x24, 0x2A)
CARET = (0x4C, 0xC2, 0xFF)

def seg(px, py, ax, ay, bx, by):
    dx, dy = bx - ax, by - ay
    t = max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)))
    return math.hypot(px - ax - t * dx, py - ay - t * dy)

def arc(px, py, cx, cy, r, a0, a1):
    a = math.atan2(py - cy, px - cx)
    if a0 <= a <= a1:
        return abs(math.hypot(px - cx, py - cy) - r)
    return min(math.hypot(px - cx - r * math.cos(t), py - cy - r * math.sin(t)) for t in (a0, a1))

def glyph(x, y):
    d = min(
        seg(x, y, 0.375, 0.355, 0.375, 0.775),                         # 竖
        arc(x, y, 0.515, 0.355, 0.14, -math.pi, -math.pi * 0.36),      # 顶上的钩
        seg(x, y, 0.25, 0.50, 0.535, 0.50),                            # 横
    )
    return d - 0.052

def box(x, y, r=0.225):
    qx, qy = abs(x - 0.5) - (0.5 - r), abs(y - 0.5) - (0.5 - r)
    return math.hypot(max(qx, 0), max(qy, 0)) + min(max(qx, qy), 0) - r

def png(n):
    rows = bytearray()
    for j in range(n):
        rows.append(0)
        for i in range(n):
            x, y = (i + 0.5) / n, (j + 0.5) / n
            a = max(0.0, min(1.0, 0.5 - box(x, y) * n))
            g = max(0.0, min(1.0, 0.5 - glyph(x, y) * n))
            k = max(0.0, min(1.0, 0.5 - (seg(x, y, 0.72, 0.30, 0.72, 0.775) - 0.04) * n))
            rgb = [c + (255 - c) * g for c in BG]
            rgb = [c + (t - c) * k for c, t in zip(rgb, CARET)]
            rows += bytes(round(c) for c in rgb) + bytes([round(a * 255)])
    def chunk(tag, data):
        return struct.pack('>I', len(data)) + tag + data + struct.pack('>I', zlib.crc32(tag + data))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', n, n, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(bytes(rows), 9)) + chunk(b'IEND', b''))

sizes = [16, 20, 24, 32, 40, 48, 64, 256]
images = [png(n) for n in sizes]
out = struct.pack('<HHH', 0, 1, len(sizes))
offset = 6 + 16 * len(sizes)
for n, img in zip(sizes, images):
    out += struct.pack('<BBBBHHII', n % 256, n % 256, 0, 0, 1, 32, len(img), offset)
    offset += len(img)
open(sys.argv[1], 'wb').write(out + b''.join(images))
open(sys.argv[2], 'wb').write(images[-1])
