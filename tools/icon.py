# 生成 assets/frun.ico：鲑鱼红的圆角方块上一条跃起的白色三文鱼。纯标准库。
# 鱼先在"拉直"的坐标里用宽度曲线描出轮廓，再整体沿一段圆弧弯过去；
# 之后按扫描线填充、每像素多次采样，每个尺寸单独画一遍。
# 用法：python tools/icon.py [预览.png]
import math, struct, sys, zlib

TOP, BOTTOM = (0xFF, 0x94, 0x6F), (0xE2, 0x45, 0x2D)   # 底色：左上到右下
WHITE = (255, 255, 255)
SIZES = (16, 20, 24, 32, 40, 48, 64, 256)

L = 210.0          # 体长（吻端到尾叉）
BODY = 0.80        # 身体占全长的比例，之后是尾鳍
W_BACK, W_BELLY = 20.0, 25.0
PED = 6.0          # 尾柄半宽
RADIUS, TILT, SCALE, CX, CY = 160.0, math.radians(-40), 1.10, 122.0, 124.0

def profile(x, w):
    """身体半宽：吻端圆润，中前部最宽，往尾柄平顺收窄。"""
    u = max(0.0, min(1.0, x / (L * BODY)))
    shape = (u ** 0.66) * ((1 - u) ** 1.05)
    peak = (0.386 ** 0.66) * (0.614 ** 1.05)
    return w * shape / peak + PED * u ** 1.5

def back(x):
    return profile(x, W_BACK)

def belly(x):
    return -profile(x, W_BELLY)

def cubic(p0, p1, p2, p3, n=18):
    out = []
    for i in range(n + 1):
        t = i / n
        a, b, c, d = (1 - t) ** 3, 3 * t * (1 - t) ** 2, 3 * t * t * (1 - t), t ** 3
        out.append((a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0], a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1]))
    return out

def body():
    xb = L * BODY
    top = [(xb * i / 60, back(xb * i / 60)) for i in range(61)]
    bottom = [(xb * i / 60, belly(xb * i / 60)) for i in range(60, -1, -1)]
    # 尾鳍：两片向后展开的叶，前缘内凹，后缘是一道月牙
    up, down, notch = (L - 3, 37.0), (L - 9, -35.0), (L * 0.895, 1.0)
    tail = (cubic((xb, PED), (xb + 10, PED + 8), (xb + 22, 30), up)
            + cubic(up, (L - 8, 24), (L * 0.90, 13), notch)
            + cubic(notch, (L * 0.90, -12), (L - 13, -22), down)
            + cubic(down, (xb + 19, -28), (xb + 9, -PED - 8), (xb, -PED)))
    return top + tail + bottom

def fin(x0, x1, height, side, skew):
    """贴着身体的一片鳍：两头贴着身体平顺地起落，顶端是圆的。"""
    edge = back if side > 0 else belly
    out = []
    for i in range(33):
        u = i / 32
        lift = math.sin(math.pi * u ** skew) ** 1.6
        x = x0 + (x1 - x0) * u
        out.append((x, edge(x) + side * (height * lift - 2)))
    return out

def dorsal(x0, x1, height):
    """背鳍：前缘斜着升上去，顶端圆，后缘内凹着落回背上。"""
    tip = (x0 + (x1 - x0) * 0.62, back(x0 + (x1 - x0) * 0.62) + height)
    return (cubic((x0, back(x0) - 2), (x0 + 8, back(x0) + 1), (tip[0] - 12, tip[1] + 1), tip)
            + cubic(tip, (tip[0] + 5, tip[1] - 1), (x1 - 9, back(x1) + 5), (x1, back(x1) - 2)))

def bend(points):
    """把拉直的鱼沿圆弧弯过去（背在外侧），转到抬头的角度，放进 256 的画板。吻端在右。"""
    out = []
    c, s = math.cos(TILT), math.sin(TILT)
    for x, y in points:
        a = (L / 2 - x) / RADIUS
        px, py = (RADIUS + y) * math.sin(a), RADIUS - (RADIUS + y) * math.cos(a)
        out.append((CX + (px * c - py * s) * SCALE, CY + (px * s + py * c) * SCALE))
    return out

def disc(cx, cy, r, n=24):
    return [(cx + r * math.cos(i / n * math.tau), cy + r * math.sin(i / n * math.tau)) for i in range(n)]

def stroke(line, half):
    """把一条折线加粗成多边形。"""
    left, right = [], []
    for i, (x, y) in enumerate(line):
        (ax, ay), (bx, by) = line[max(i - 1, 0)], line[min(i + 1, len(line) - 1)]
        d = math.hypot(bx - ax, by - ay)
        nx, ny = -(by - ay) / d * half, (bx - ax) / d * half
        left.append((x + nx, y + ny))
        right.append((x - nx, y - ny))
    return left + right[::-1]

def shapes(size):
    """[(多边形, 颜色或 None 表示底色, 不透明度)]，坐标在 256 的画板里。小尺寸只留剪影。"""
    out = [(bend(body()), WHITE, 1.0),
           (bend(dorsal(74, 124, 16)), WHITE, 1.0),
           (bend(fin(92, 122, 13, -1, 0.80)), WHITE, 1.0)]     # 腹鳍：腹面只留这一片
    if size >= 48:
        gill = bend(cubic((44, back(44) * 0.74), (37, 7), (38, -8), (47, belly(47) * 0.76)))
        out.append((stroke(gill, 1.3), None, 0.55))
    if size >= 32:
        ex, ey = bend([(24, 4.5)])[0]
        out.append((disc(ex, ey, 4.6 if size >= 48 else 6.0), None, 1.0))
    return out

def box(x, y, r=58 / 256):
    qx, qy = abs(x - 0.5) - (0.5 - r), abs(y - 0.5) - (0.5 - r)
    return math.hypot(max(qx, 0), max(qy, 0)) + min(max(qx, qy), 0) - r

def image(size):
    ss = 4 if size >= 256 else 8
    n = size * ss
    k = n / 256
    paint = [[None] * n for _ in range(n)]          # None 表示底色，否则 (颜色或 None, 不透明度)
    for pts, color, alpha in shapes(size):
        pts = [(x * k, y * k) for x, y in pts]
        for j in range(n):
            y = j + 0.5
            xs = sorted(x0 + (y - y0) * (x1 - x0) / (y1 - y0)
                        for (x0, y0), (x1, y1) in zip(pts, pts[1:] + pts[:1])
                        if (y0 <= y < y1) or (y1 <= y < y0))
            for a, b in zip(xs[0::2], xs[1::2]):
                for i in range(max(0, int(a + 0.5)), min(n, int(b + 0.5))):
                    paint[j][i] = (color, alpha)
    rows = bytearray()
    for j in range(size):
        rows.append(0)
        for i in range(size):
            x, y = (i + 0.5) / size, (j + 0.5) / size
            t = (x + y) / 2
            ground = [a + (b - a) * t for a, b in zip(TOP, BOTTOM)]
            acc = [0.0, 0.0, 0.0]
            for sj in range(ss):
                for cell in paint[j * ss + sj][i * ss:(i + 1) * ss]:
                    if cell is None:
                        c = ground
                    elif cell[0] is None:                   # 底色盖在白色上
                        c = [255 + (g - 255) * cell[1] for g in ground]
                    else:
                        c = cell[0]
                    for q in range(3):
                        acc[q] += c[q]
            cover = max(0.0, min(1.0, 0.5 - box(x, y) * size))
            rows += bytes([round(v / (ss * ss)) for v in acc] + [round(cover * 255)])
    chunk = lambda tag, data: struct.pack('>I', len(data)) + tag + data + struct.pack('>I', zlib.crc32(tag + data))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(bytes(rows), 9)) + chunk(b'IEND', b''))

if __name__ == '__main__':
    if len(sys.argv) > 1:
        open(sys.argv[1], 'wb').write(image(int(sys.argv[2]) if len(sys.argv) > 2 else 256))
        sys.exit()
    images = [image(s) for s in SIZES]
    head, offset = struct.pack('<HHH', 0, 1, len(images)), 6 + 16 * len(images)
    for s, data in zip(SIZES, images):
        head += struct.pack('<BBBBHHII', s % 256, s % 256, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    open('assets/frun.ico', 'wb').write(head + b''.join(images))
