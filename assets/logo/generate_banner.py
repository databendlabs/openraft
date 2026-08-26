#!/usr/bin/env python3
"""Generate the OpenRaft banner: a centred mark-and-wordmark lockup.

GitHub lays out a README heading and an inline image on separate lines, so the
lockup cannot be built from HTML. This banner is one image instead, drawn at the
width of a README column, so it arrives on the page already composed.

The word is outlined rather than set as SVG text, because an SVG loaded through
an `img` tag resolves fonts on the reader's machine and would otherwise render
differently on every platform.

Writes openraft-banner.svg and openraft-banner-dark.svg next to this script.
"""

import math
import pathlib
import re

import generate_mark
from generate_mark import NEUTRAL_ON_DARK, NEUTRAL_ON_LIGHT

WIDTH = 900
HEIGHT = 180
MARK_SIZE = 140
# Visible space between the mark's ring and the wordmark.
GAP = 26.5
WORD_SIZE = 34

# "OpenRaft" outlined from TeX Gyre Heros Bold, a GUST Font License grotesque.
# The path sits on a 1000-unit em with its baseline at y = 0. Its ink edges come
# from `word_ink_x` rather than from the font's advance, so the visible shape
# can be centred.
WORD_CAP_HEIGHT = 729
WORD_PATH = (
    "M742.0 -360.0C742.0 -580.0 603.0 -741.0 390.0 -741.0C176.0 -741.0 40.0 -574.0 40.0 -365.0C40.0 -147.0 177.0 12.0 391.0 12.0C603.0 12.0 742.0 -147.0 742.0 -360.0ZM592.0 -361.0C592.0 -212.0 511.0 -116.0 391.0 -116.0C270.0 -116.0 190.0 -212.0 190.0 -365.0C190.0 -512.0 270.0 -613.0 391.0 -613.0C514.0 -613.0 592.0 -513.0 592.0 -361.0ZM1352.0 -270.0C1352.0 -430.0 1267.0 -549.0 1126.0 -549.0C1058.0 -549.0 1010.0 -520.0 976.0 -460.0V-540.0H836.0V218.0H976.0V-72.0C1010.0 -12.0 1058.0 9.0 1126.0 9.0C1256.0 9.0 1352.0 -106.0 1352.0 -270.0ZM1212.0 -268.0C1212.0 -170.0 1163.0 -108.0 1094.0 -108.0C1024.0 -108.0 976.0 -169.0 976.0 -270.0C976.0 -371.0 1024.0 -432.0 1094.0 -432.0C1165.0 -432.0 1212.0 -371.0 1212.0 -268.0ZM1914.0 -250.0C1914.0 -436.0 1820.0 -549.0 1661.0 -549.0C1506.0 -549.0 1411.0 -444.0 1411.0 -263.0C1411.0 -90.0 1505.0 9.0 1658.0 9.0C1779.0 9.0 1877.0 -46.0 1908.0 -152.0H1770.0C1758.0 -113.0 1714.0 -98.0 1663.0 -98.0C1597.0 -98.0 1555.0 -128.0 1551.0 -233.0H1913.0ZM1768.0 -326.0H1553.0C1562.0 -401.0 1597.0 -442.0 1659.0 -442.0C1719.0 -442.0 1761.0 -403.0 1768.0 -326.0ZM2491.0 0.0V-362.0C2491.0 -481.0 2425.0 -549.0 2310.0 -549.0C2237.0 -549.0 2188.0 -522.0 2148.0 -462.0V-540.0H2008.0V0.0H2148.0V-324.0C2148.0 -388.0 2193.0 -430.0 2261.0 -430.0C2321.0 -430.0 2351.0 -397.0 2351.0 -333.0V0.0ZM3233.0 0.0V-27.0C3210.0 -40.0 3201.0 -55.0 3201.0 -87.0C3197.0 -302.0 3193.0 -312.0 3100.0 -352.0C3182.0 -384.0 3223.0 -443.0 3223.0 -532.0C3223.0 -645.0 3156.0 -729.0 3027.0 -729.0H2636.0V0.0H2786.0V-289.0H2958.0C3023.0 -289.0 3051.0 -263.0 3051.0 -202.0L3049.0 -125.0C3049.0 -57.0 3053.0 -35.0 3072.0 0.0ZM3073.0 -511.0C3073.0 -446.0 3052.0 -414.0 2967.0 -414.0H2786.0V-604.0H2967.0C3049.0 -604.0 3073.0 -574.0 3073.0 -511.0ZM3802.0 0.0V-17.0C3777.0 -40.0 3770.0 -55.0 3770.0 -83.0V-383.0C3770.0 -493.0 3695.0 -549.0 3549.0 -549.0C3403.0 -549.0 3327.0 -487.0 3318.0 -362.0H3453.0C3460.0 -418.0 3483.0 -436.0 3552.0 -436.0C3606.0 -436.0 3633.0 -418.0 3633.0 -382.0C3633.0 -325.0 3591.0 -331.0 3521.0 -319.0L3465.0 -309.0C3358.0 -290.0 3306.0 -244.0 3306.0 -146.0C3306.0 -41.0 3380.0 9.0 3470.0 9.0C3530.0 9.0 3585.0 -17.0 3634.0 -68.0C3634.0 -40.0 3637.0 -16.0 3650.0 0.0ZM3633.0 -231.0C3633.0 -150.0 3593.0 -104.0 3522.0 -104.0C3475.0 -104.0 3446.0 -122.0 3446.0 -162.0C3446.0 -203.0 3468.0 -218.0 3526.0 -229.0L3574.0 -238.0C3611.0 -245.0 3617.0 -247.0 3633.0 -255.0ZM4147.0 -436.0V-529.0H4064.0V-582.0C4064.0 -610.0 4076.0 -624.0 4102.0 -624.0C4114.0 -624.0 4130.0 -623.0 4142.0 -621.0V-726.0C4116.0 -728.0 4081.0 -729.0 4062.0 -729.0C3969.0 -729.0 3924.0 -685.0 3924.0 -594.0V-529.0H3848.0V-436.0H3924.0V0.0H4064.0V-436.0ZM4468.0 0.0V-98.0C4454.0 -96.0 4446.0 -95.0 4436.0 -95.0C4399.0 -95.0 4390.0 -106.0 4390.0 -154.0V-436.0H4468.0V-529.0H4390.0V-674.0H4250.0V-529.0H4181.0V-436.0H4250.0V-116.0C4250.0 -31.0 4295.0 4.0 4387.0 4.0C4418.0 4.0 4443.0 1.0 4468.0 0.0Z"
)

INK_ON_LIGHT = "#5e646a"
INK_ON_DARK = "#b2b8bf"


def _quadratic_roots(a, b, c):
    """The real roots of `a*t^2 + b*t + c`; none when the equation is constant."""
    if a == 0:
        return [-c / b] if b != 0 else []
    discriminant = b * b - 4 * a * c
    if discriminant < 0:
        return []
    root = math.sqrt(discriminant)
    return [(-b + root) / (2 * a), (-b - root) / (2 * a)]


def _cubic_x(x0, x1, x2, x3, t):
    """The x of the cubic Bezier through `x0..x3` at parameter `t`."""
    rest = 1 - t
    return (rest ** 3 * x0 + 3 * rest ** 2 * t * x1
            + 3 * rest * t ** 2 * x2 + t ** 3 * x3)


def _cubic_extreme_xs(x0, x1, x2, x3):
    """Every x at which the cubic Bezier through `x0..x3` can be extreme.

    A cubic is leftmost or rightmost either at an end point or where its
    derivative in x vanishes, so those points are the only candidates.
    """
    a = 3 * (-x0 + 3 * x1 - 3 * x2 + x3)
    b = 6 * (x0 - 2 * x1 + x2)
    c = 3 * (x1 - x0)
    inside = [t for t in _quadratic_roots(a, b, c) if 0 < t < 1]
    return [x0, x3] + [_cubic_x(x0, x1, x2, x3, t) for t in inside]


def word_ink_x():
    """The leftmost and rightmost ink of WORD_PATH, in em units.

    The two edges are read from the outline itself so that re-outlining the
    word at another weight, or outlining another word, cannot leave a stale
    number behind. `V` needs no branch below because it does not move x.
    """
    if not re.fullmatch(r"(?:[MLHVCZ][-\d. ]*)+", WORD_PATH):
        raise ValueError("WORD_PATH holds a command this routine cannot measure")
    xs = []
    x = 0.0
    subpath_start = 0.0
    for command, payload in re.findall(r"([MLHVCZ])([-\d. ]*)", WORD_PATH):
        args = [float(v) for v in payload.replace("-", " -").split()]
        if command in "ML":
            for i in range(0, len(args), 2):
                x = args[i]
                xs.append(x)
            if command == "M":
                subpath_start = args[0]
        elif command == "H":
            for value in args:
                x = value
                xs.append(x)
        elif command == "C":
            for i in range(0, len(args), 6):
                xs.extend(_cubic_extreme_xs(x, args[i], args[i + 2], args[i + 4]))
                x = args[i + 4]
        elif command == "Z":
            x = subpath_start
    return min(xs), max(xs)


def render(neutral, ink):
    mark_scale = MARK_SIZE / generate_mark.VIEW
    word_scale = WORD_SIZE / 1000
    mark_ink_left = ((generate_mark.CENTRE - generate_mark.RING_OUTER)
                     * mark_scale)
    mark_ink_width = 2 * generate_mark.RING_OUTER * mark_scale
    ink_left, ink_right = word_ink_x()
    word_ink_left = ink_left * word_scale
    word_ink_width = (ink_right - ink_left) * word_scale
    lockup_ink_width = mark_ink_width + GAP + word_ink_width
    lockup_ink_x = (WIDTH - lockup_ink_width) / 2
    mark_x = lockup_ink_x - mark_ink_left
    mark_y = (HEIGHT - MARK_SIZE) / 2
    word_x = lockup_ink_x + mark_ink_width + GAP - word_ink_left
    # sit the word so its capitals centre on the same line as the mark
    word_baseline = HEIGHT / 2 + WORD_CAP_HEIGHT * word_scale / 2
    mark_paths = "\n".join(f'    <path d="{d}" fill="{fill}"/>'
                           for d, fill in generate_mark.paths(neutral))
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {WIDTH} {HEIGHT}" '
            f'width="{WIDTH}" height="{HEIGHT}" role="img" aria-label="OpenRaft">\n'
            f'  <title>OpenRaft</title>\n'
            f'  <g transform="translate({mark_x} {mark_y}) scale({mark_scale})">\n'
            f'{mark_paths}\n'
            f'  </g>\n'
            f'  <path transform="translate({word_x} {word_baseline}) scale({word_scale})" '
            f'd="{WORD_PATH}" fill="{ink}"/>\n'
            f'</svg>\n')


if __name__ == "__main__":
    here = pathlib.Path(__file__).parent
    for name, neutral, ink in (("openraft-banner.svg", NEUTRAL_ON_LIGHT, INK_ON_LIGHT),
                               ("openraft-banner-dark.svg", NEUTRAL_ON_DARK, INK_ON_DARK)):
        (here / name).write_text(render(neutral, ink))
        print(f"wrote {name}")
