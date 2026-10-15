#!/usr/bin/env python3
"""Генерирует анимацию заставки (Lottie .tgs) из одноцветного знака docs/brand/taurigram-glyph.svg.

Анимация повторяет заставку Ethernet: появление, лёгкое покачивание и затухание за 3 секунды.
Поддерживаются команды путей M, L, H, V, C, Z (абсолютные).

Использование: generate-splash.py <glyph.svg> <out.tgs> [preview.svg]
"""

import gzip
import json
import re
import sys

CANVAS = 512
GLYPH_BOX = 480  # Знак вписывается в квадрат 480×480 вокруг центра холста
FPS = 60
DURATION_FRAMES = 180

TOKEN_RE = re.compile(r'[MLHVCZmlhvcz]|-?\d*\.?\d+(?:e-?\d+)?')


def parse_path(d):
    """Возвращает список контуров: [(vertices, in_tangents, out_tangents, closed)]"""
    tokens = TOKEN_RE.findall(d)
    contours = []
    verts, ins, outs = [], [], []
    pos = 0
    cur = (0.0, 0.0)
    cmd = None

    def num():
        nonlocal pos
        value = float(tokens[pos])
        pos += 1
        return value

    def finish(closed):
        nonlocal verts, ins, outs
        if not verts:
            return
        # Если контур замкнут кривой в начальную точку — сливаем совпадающие вершины
        if closed and len(verts) > 1 and abs(verts[-1][0] - verts[0][0]) < 1e-6 and abs(verts[-1][1] - verts[0][1]) < 1e-6:
            ins[0] = ins[-1]
            verts.pop()
            ins.pop()
            outs.pop()
        contours.append((verts, ins, outs, closed))
        verts, ins, outs = [], [], []

    while pos < len(tokens):
        token = tokens[pos]
        if re.fullmatch(r'[A-Za-z]', token):
            cmd = token
            pos += 1
            if cmd in 'Zz':
                finish(True)
                continue
        if cmd == 'M':
            finish(False)
            cur = (num(), num())
            verts.append(cur)
            ins.append((0.0, 0.0))
            outs.append((0.0, 0.0))
            cmd = 'L'
        elif cmd in ('L', 'H', 'V'):
            if cmd == 'L':
                cur = (num(), num())
            elif cmd == 'H':
                cur = (num(), cur[1])
            else:
                cur = (cur[0], num())
            verts.append(cur)
            ins.append((0.0, 0.0))
            outs.append((0.0, 0.0))
        elif cmd == 'C':
            c1 = (num(), num())
            c2 = (num(), num())
            end = (num(), num())
            prev = verts[-1]
            outs[-1] = (c1[0] - prev[0], c1[1] - prev[1])
            verts.append(end)
            ins.append((c2[0] - end[0], c2[1] - end[1]))
            outs.append((0.0, 0.0))
            cur = end
        else:
            raise ValueError(f'Unsupported path command: {cmd}')
    finish(False)
    return contours


def load_glyph(svg_path):
    svg = open(svg_path, encoding='utf-8').read()
    view_box = [float(v) for v in re.search(r'viewBox="([^"]+)"', svg).group(1).split()]
    paths = re.findall(r'<path[^>]*\sd="([^"]+)"', svg)
    return view_box, [c for d in paths for c in parse_path(d)]


def to_lottie_shapes(view_box, contours):
    vx, vy, vw, vh = view_box
    scale = GLYPH_BOX / max(vw, vh)
    offset_x = CANVAS / 2 - (vx + vw / 2) * scale
    offset_y = CANVAS / 2 - (vy + vh / 2) * scale

    def point(p):
        return [round(p[0] * scale + offset_x, 3), round(p[1] * scale + offset_y, 3)]

    def tangent(t):
        return [round(t[0] * scale, 3), round(t[1] * scale, 3)]

    shapes = []
    for index, (verts, ins, outs, closed) in enumerate(contours):
        shapes.append({
            'ty': 'sh',
            'nm': f'Path {index + 1}',
            'ks': {'a': 0, 'k': {
                'v': [point(v) for v in verts],
                'i': [tangent(t) for t in ins],
                'o': [tangent(t) for t in outs],
                'c': closed,
            }},
        })
    return shapes, (scale, offset_x, offset_y)


def keyframes(frames, ease_in=(0.833, 0.833), ease_out=(0.167, 0.167)):
    result = []
    for i, (frame, value) in enumerate(frames):
        key = {'t': frame, 's': [value]}
        if i < len(frames) - 1:
            key['i'] = {'x': [ease_in[0]], 'y': [ease_in[1]]}
            key['o'] = {'x': [ease_out[0]], 'y': [ease_out[1]]}
        result.append(key)
    return {'a': 1, 'k': result}


def build_animation(shapes):
    group = {
        'ty': 'gr',
        'nm': 'Glyph',
        'it': shapes + [
            {'ty': 'fl', 'nm': 'Fill', 'c': {'a': 0, 'k': [1, 1, 1, 1]}, 'o': {'a': 0, 'k': 100}, 'r': 1},
            {'ty': 'tr', 'nm': 'Transform', 'p': {'a': 0, 'k': [0, 0]}, 'a': {'a': 0, 'k': [0, 0]},
             's': {'a': 0, 'k': [100, 100]}, 'r': {'a': 0, 'k': 0}, 'o': {'a': 0, 'k': 100},
             'sk': {'a': 0, 'k': 0}, 'sa': {'a': 0, 'k': 0}},
        ],
    }
    half = CANVAS / 2
    layer = {
        'ddd': 0, 'ind': 1, 'ty': 4, 'nm': 'Taurigram', 'sr': 1,
        'ks': {
            'o': keyframes([(0, 0), (40, 100), (120, 100), (DURATION_FRAMES - 1, 0)]),
            'r': keyframes([(0, -1.7), (50, -5), (130, 5), (DURATION_FRAMES - 1, -2)], ease_in=(0.667, 1)),
            'p': {'a': 0, 'k': [half, half, 0]},
            'a': {'a': 0, 'k': [half, half, 0]},
            # Как у Ethernet: знак занимает около половины кадра и слегка «дышит»
            's': keyframes([(0, 46), (60, 50), (120, 50), (DURATION_FRAMES - 1, 48)], ease_in=(0.667, 1)),
        },
        'ao': 0, 'shapes': [group], 'ip': 0, 'op': DURATION_FRAMES, 'st': 0, 'bm': 0,
    }
    # Lottie ждёт масштаб как [x, y, z]
    for key in layer['ks']['s']['k']:
        key['s'] = [key['s'][0], key['s'][0], 100]
    return {
        'tgs': 1, 'v': '5.5.2', 'fr': FPS, 'ip': 0, 'op': DURATION_FRAMES,
        'w': CANVAS, 'h': CANVAS, 'nm': 'Taurigram splash', 'ddd': 0, 'assets': [], 'layers': [layer],
    }


def preview_svg(shapes):
    """Обратная конвертация контуров Lottie в SVG — для визуальной проверки"""
    parts = []
    for shape in shapes:
        k = shape['ks']['k']
        v, i, o = k['v'], k['i'], k['o']
        d = [f'M{v[0][0]} {v[0][1]}']
        count = len(v) if k['c'] else len(v) - 1
        for n in range(count):
            a, b = v[n], v[(n + 1) % len(v)]
            c1 = (a[0] + o[n][0], a[1] + o[n][1])
            c2 = (b[0] + i[(n + 1) % len(v)][0], b[1] + i[(n + 1) % len(v)][1])
            d.append(f'C{c1[0]} {c1[1]} {c2[0]} {c2[1]} {b[0]} {b[1]}')
        if k['c']:
            d.append('Z')
        parts.append(f'<path d="{" ".join(d)}"/>')
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {CANVAS} {CANVAS}">'
            f'<rect width="{CANVAS}" height="{CANVAS}" fill="#000"/><g fill="#fff">{"".join(parts)}</g></svg>')


def main():
    glyph_path, out_path = sys.argv[1], sys.argv[2]
    view_box, contours = load_glyph(glyph_path)
    shapes, _ = to_lottie_shapes(view_box, contours)
    with gzip.open(out_path, 'wb') as f:
        f.write(json.dumps(build_animation(shapes), separators=(',', ':')).encode())
    if len(sys.argv) > 3:
        open(sys.argv[3], 'w', encoding='utf-8').write(preview_svg(shapes))


if __name__ == '__main__':
    main()
