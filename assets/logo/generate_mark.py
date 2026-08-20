#!/usr/bin/env python3
"""Generate the OpenRaft mark.

The mark is a ring of five segments around a core. Three segments carry the
brand teal and form the quorum; the remaining two stay neutral. The third
quorum segment is split into three bands that step darker, and the core is cut
by four rays that line up with the edges of those bands.

Every white gap in the mark is one width, `SLOT`. Because the gaps are drawn as
parallel offsets from their dividing line rather than as radial cuts, a gap
measures the same at the ring's inner edge as at its outer edge.

Writes openraft-mark.svg and openraft-mark-dark.svg next to this script.
"""

import math
import pathlib

# Geometry, in the units of the 64x64 viewBox.
VIEW = 64
CENTRE = 32.0
RING_OUTER = 26.0
RING_INNER = 15.0
CORE_RADIUS = 11.0
CORE_HOLE = 3.6
CORE_FILLET = 1.0
SLOT = 2.0

SEGMENT_SPAN = 72.0
NEUTRAL_STARTS = [18.0, 90.0]
QUORUM_STARTS = [162.0, 234.0]
BAND_EDGES = [306.0, 330.0, 354.0, 378.0]

# Colour, as oklch(lightness, chroma, hue).
RING_HUE = 190.0
RING_CHROMA = 0.128
RING_LIGHTNESS = 0.60
BAND_LIGHTNESS = [0.66, 0.60, 0.545]
CORE = (0.64, 0.13, 265.0)
CHIPS = [(0.84, 0.165, 88.0), (0.585, 0.215, 28.0), (0.78, 0.19, 148.0)]

# The two segments that have not agreed, flattened against each background.
NEUTRAL_ON_LIGHT = "#d5d6d7"
NEUTRAL_ON_DARK = "#383b3f"


def _srgb_transfer(channel):
    if channel <= 0.0031308:
        return 12.92 * channel
    return 1.055 * channel ** (1 / 2.4) - 0.055


def oklch(lightness, chroma, hue):
    """Convert an oklch colour to an sRGB hex string, clipping out-of-gamut channels."""
    a = chroma * math.cos(math.radians(hue))
    b = chroma * math.sin(math.radians(hue))
    long_ = (lightness + 0.3963377774 * a + 0.2158037573 * b) ** 3
    medium = (lightness - 0.1055613458 * a - 0.0638541728 * b) ** 3
    short = (lightness - 0.0894841775 * a - 1.2914855480 * b) ** 3
    red = 4.0767416621 * long_ - 3.3077115913 * medium + 0.2309699292 * short
    green = -1.2684380046 * long_ + 2.6097574011 * medium - 0.3413193965 * short
    blue = -0.0041960863 * long_ - 0.7034186147 * medium + 1.7076147010 * short
    channels = [_srgb_transfer(v) for v in (red, green, blue)]
    return "#" + "".join(f"{round(min(1.0, max(0.0, v)) * 255):02x}" for v in channels)


def point(angle, radius):
    """A point on the circle of the given radius, rounded to two decimals."""
    x = CENTRE + radius * math.cos(math.radians(angle))
    y = CENTRE + radius * math.sin(math.radians(angle))
    return round(x, 2), round(y, 2)


def _offset(gap, radius):
    """The angle by which a `gap`-deep parallel offset shifts the edge at `radius`."""
    if gap == 0:
        return 0.0
    return math.degrees(math.asin(gap / radius))


def sector(start, end, inner, outer, gap_start, gap_end):
    """An annular sector whose two straight edges are parallel offsets of their rays."""
    a_inner = start + _offset(gap_start, inner)
    a_outer = start + _offset(gap_start, outer)
    b_outer = end - _offset(gap_end, outer)
    b_inner = end - _offset(gap_end, inner)
    p_inner_start, p_outer_start = point(a_inner, inner), point(a_outer, outer)
    p_outer_end, p_inner_end = point(b_outer, outer), point(b_inner, inner)
    long_outer = 1 if (b_outer - a_outer) > 180 else 0
    long_inner = 1 if (b_inner - a_inner) > 180 else 0
    return (f"M{p_inner_start[0]} {p_inner_start[1]}L{p_outer_start[0]} {p_outer_start[1]}"
            f"A{outer} {outer} 0 {long_outer} 1 {p_outer_end[0]} {p_outer_end[1]}"
            f"L{p_inner_end[0]} {p_inner_end[1]}"
            f"A{inner} {inner} 0 {long_inner} 0 {p_inner_start[0]} {p_inner_start[1]}Z")


def pointed_wedge(start, end, radius, gap):
    """A wedge narrow enough that its two offset edges meet before reaching the hole."""
    half_span = (end - start) / 2
    tip = point((start + end) / 2, gap / math.sin(math.radians(half_span)))
    edge = _offset(gap, radius)
    arc_start, arc_end = point(start + edge, radius), point(end - edge, radius)
    long_arc = 1 if (end - edge) - (start + edge) > 180 else 0
    return (f"M{tip[0]} {tip[1]}L{arc_start[0]} {arc_start[1]}"
            f"A{radius} {radius} 0 {long_arc} 1 {arc_end[0]} {arc_end[1]}"
            f"L{tip[0]} {tip[1]}Z")


def largest_fillet(start, end, hole, gap):
    """The largest inner-corner radius that keeps a wedge's two fillets apart."""
    low, high = 0.0, 3.0
    half_span = (end - start) / 2
    for _ in range(40):
        candidate = (low + high) / 2
        reach = candidate + gap
        discriminant = (hole + candidate) ** 2 - reach ** 2
        fits = discriminant > 0 and math.degrees(
            math.atan2(reach, math.sqrt(discriminant))) < half_span - 0.5
        low, high = (candidate, high) if fits else (low, candidate)
    return low


def holed_wedge(start, end, radius, gap, hole, requested_fillet):
    """A wedge truncated by the centre hole, with both inner corners rounded."""
    fillet = min(requested_fillet, largest_fillet(start, end, hole, gap))
    if fillet < 0.3:
        return sector(start, end, hole, radius, gap, gap)
    ray_start = (math.cos(math.radians(start)), math.sin(math.radians(start)))
    ray_end = (math.cos(math.radians(end)), math.sin(math.radians(end)))
    normal_start = (-ray_start[1], ray_start[0])
    normal_end = (ray_end[1], -ray_end[0])
    along = math.sqrt((hole + fillet) ** 2 - (fillet + gap) ** 2)
    place = lambda ray, normal, out: (CENTRE + along * ray[0] + out * normal[0],
                                      CENTRE + along * ray[1] + out * normal[1])
    tangent_start, centre_start = place(ray_start, normal_start, gap), place(ray_start, normal_start, fillet + gap)
    tangent_end, centre_end = place(ray_end, normal_end, gap), place(ray_end, normal_end, fillet + gap)
    pull_in = lambda c: (CENTRE + (c[0] - CENTRE) * hole / (hole + fillet),
                         CENTRE + (c[1] - CENTRE) * hole / (hole + fillet))
    hole_start, hole_end = pull_in(centre_start), pull_in(centre_end)

    def sweep(centre, first, second):
        u = (first[0] - centre[0], first[1] - centre[1])
        v = (second[0] - centre[0], second[1] - centre[1])
        return 1 if u[0] * v[1] - u[1] * v[0] > 0 else 0

    edge = _offset(gap, radius)
    arc_start, arc_end = point(start + edge, radius), point(end - edge, radius)
    long_arc = 1 if (end - edge) - (start + edge) > 180 else 0
    corner = math.degrees(math.atan2(fillet + gap, along))
    long_hole = 1 if (end - corner) - (start + corner) > 180 else 0
    at = lambda p: f"{round(p[0], 2)} {round(p[1], 2)}"
    return (f"M{at(tangent_start)}L{arc_start[0]} {arc_start[1]}"
            f"A{radius} {radius} 0 {long_arc} 1 {arc_end[0]} {arc_end[1]}"
            f"L{at(tangent_end)}"
            f"A{fillet} {fillet} 0 0 {sweep(centre_end, tangent_end, hole_end)} {at(hole_end)}"
            f"A{hole} {hole} 0 {long_hole} 0 {at(hole_start)}"
            f"A{fillet} {fillet} 0 0 {sweep(centre_start, hole_start, tangent_start)} {at(tangent_start)}Z")


def core_wedge(start, end, gap):
    """Pointed when the wedge is too narrow to reach the hole, truncated when it is not."""
    tip_radius = gap / math.sin(math.radians((end - start) / 2))
    if tip_radius > CORE_HOLE:
        return pointed_wedge(start, end, CORE_RADIUS, gap)
    return holed_wedge(start, end, CORE_RADIUS, gap, CORE_HOLE, CORE_FILLET)


def paths(neutral):
    """Every path of the mark, in paint order, against the given neutral grey."""
    gap = SLOT / 2
    quorum = oklch(RING_LIGHTNESS, RING_CHROMA, RING_HUE)
    out = []
    for start in NEUTRAL_STARTS:
        out.append((sector(start, start + SEGMENT_SPAN, RING_INNER, RING_OUTER, gap, gap), neutral))
    for start in QUORUM_STARTS:
        out.append((sector(start, start + SEGMENT_SPAN, RING_INNER, RING_OUTER, gap, gap), quorum))
    bands = list(zip(BAND_EDGES, BAND_EDGES[1:], BAND_LIGHTNESS))
    for start, end, lightness in bands:
        gap_start = gap if start == BAND_EDGES[0] else 0.0
        gap_end = gap if end == BAND_EDGES[-1] else 0.0
        colour = oklch(lightness, RING_CHROMA, RING_HUE)
        out.append((sector(start, end, RING_INNER, RING_OUTER, gap_start, gap_end), colour))
    rays = BAND_EDGES + [BAND_EDGES[0] + 360]
    spans = list(zip(rays, rays[1:]))
    for (start, end), colour in zip(spans, CHIPS + [CORE]):
        out.append((core_wedge(start, end, gap), oklch(*colour)))
    return out


def render(neutral):
    body = "\n".join(f'  <path d="{d}" fill="{fill}"/>' for d, fill in paths(neutral))
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {VIEW} {VIEW}" '
            f'width="{VIEW}" height="{VIEW}" role="img" aria-label="OpenRaft">\n'
            f'  <title>OpenRaft</title>\n{body}\n</svg>\n')


if __name__ == "__main__":
    here = pathlib.Path(__file__).parent
    for name, neutral in (("openraft-mark.svg", NEUTRAL_ON_LIGHT),
                          ("openraft-mark-dark.svg", NEUTRAL_ON_DARK)):
        (here / name).write_text(render(neutral))
        print(f"wrote {name}")
