#!/usr/bin/env python3
"""
Generate one circular-avatar SVG per contributor login.

Reads a login list (one per line, e.g. the file written by list_contributors.py)
and writes <dir>/svg/<login>.svg for each: the GitHub avatar clipped to a circle
on the left, with the @login handle vertically centered — in GitHub's own
username font — to its right. The avatar is embedded as a base64 data URI, so
each SVG is self-contained and renders inline on GitHub (which sandboxes README
images and blocks external resource loads).

Downloaded avatars are cached under <dir>/avatars/, so refreshing the list only
fetches avatars that are new; pass --refresh to re-download every one. SVGs whose
login is no longer in the list are pruned from <dir>/svg/.

Usage:
    scripts/contributors/generate_contributor_svgs.py [--dir DIR] [--txt NAME] [--refresh]

Defaults: --dir assets/contributors, --txt contributors.txt.
Requires: curl (only when fetching, i.e. new logins or --refresh).
"""

import argparse
import base64
import subprocess
import sys
from pathlib import Path

# The handle uses GitHub's own username styling: the Primer sans-serif font
# stack, the semibold weight GitHub renders @-mentions in, and its text colors.
AVATAR_PX = 32  # circle diameter
AVATAR_FETCH_PX = 64  # fetch at 2x for a crisp downscaled circle
GAP_PX = 7  # space between circle and handle
FONT_PX = 14
CHAR_PX = 8.8  # rough advance width per handle character at FONT_PX (semibold)
PAD_PX = 8  # trailing padding after the handle

STYLE = (
    "    .h { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', 'Noto Sans', "
    "Helvetica, Arial, sans-serif, 'Apple Color Emoji', 'Segoe UI Emoji'; "
    "font-size: " + str(FONT_PX) + "px; font-weight: 600; fill: #1f2328; }\n"
    "    @media (prefers-color-scheme: dark) { .h { fill: #e6edf3; } }"
)


def image_kind(data):
    """(MIME type, file extension) sniffed from an image's magic bytes.

    GitHub serves avatars in whatever format the user uploaded (PNG, JPEG, ...)
    regardless of the ``.png`` request URL, so the data URI must declare the
    real type or strict decoders refuse to render it.
    """
    if data[:8] == b"\x89PNG\r\n\x1a\n":
        return "image/png", "png"
    if data[:3] == b"\xff\xd8\xff":
        return "image/jpeg", "jpg"
    if data[:6] in (b"GIF87a", b"GIF89a"):
        return "image/gif", "gif"
    if data[:4] == b"RIFF" and data[8:12] == b"WEBP":
        return "image/webp", "webp"
    raise ValueError(f"unrecognized avatar image format, magic bytes: {data[:12]!r}")


def data_uri(data):
    """Base64 ``data:`` URI for image bytes, with the sniffed MIME type."""
    mime, _ = image_kind(data)
    return f"data:{mime};base64,{base64.b64encode(data).decode('ascii')}"


def cached_avatar(cache_dir, name):
    """Cached avatar bytes for a login, or None if not yet downloaded."""
    for path in sorted(cache_dir.glob(f"{name}.*")):
        return path.read_bytes()
    return None


def download_avatar(cache_dir, name):
    """Download a login's avatar, store it in the cache under its real extension."""
    url = f"https://github.com/{name}.png?size={AVATAR_FETCH_PX}"
    data = subprocess.run(["curl", "-fsSL", url], capture_output=True, check=True).stdout
    _, ext = image_kind(data)
    for stale in cache_dir.glob(f"{name}.*"):
        stale.unlink()
    (cache_dir / f"{name}.{ext}").write_bytes(data)
    return data


def render_svg(name, avatar_href):
    """One contributor's SVG: circular avatar left, ``@name`` centered right."""
    diameter = AVATAR_PX
    radius = diameter // 2
    handle = f"@{name}"
    text_x = diameter + GAP_PX
    width = round(text_x + CHAR_PX * len(handle) + PAD_PX)
    height = diameter
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" '
        f'width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}" role="img" aria-label="{handle}">\n'
        f"  <defs>\n"
        f'    <clipPath id="c"><circle cx="{radius}" cy="{radius}" r="{radius}"/></clipPath>\n'
        f"    <style>\n{STYLE}\n    </style>\n"
        f"  </defs>\n"
        f'  <image xlink:href="{avatar_href}" x="0" y="0" '
        f'width="{diameter}" height="{diameter}" clip-path="url(#c)"/>\n'
        f'  <text class="h" x="{text_x}" y="{radius}" dominant-baseline="central">{handle}</text>\n'
        f"</svg>\n"
    )


def write_svgs(names, svg_dir, cache_dir, refresh=False):
    """Write one embedded-avatar SVG per name, caching downloads and pruning drops.

    An avatar already in `cache_dir` is reused (no network); only new logins are
    downloaded, unless `refresh` forces every avatar to be re-fetched.
    """
    svg_dir.mkdir(parents=True, exist_ok=True)
    cache_dir.mkdir(parents=True, exist_ok=True)
    keep = {f"{name}.svg" for name in names}
    for stale in svg_dir.glob("*.svg"):
        if stale.name not in keep:
            stale.unlink()
    for i, name in enumerate(names, 1):
        data = None if refresh else cached_avatar(cache_dir, name)
        origin = "cached" if data is not None else "fetched"
        if data is None:
            data = download_avatar(cache_dir, name)
        (svg_dir / f"{name}.svg").write_text(render_svg(name, data_uri(data)))
        print(f"  [{i}/{len(names)}] {name} ({origin})", file=sys.stderr)


def read_logins(path):
    """Sorted, de-duplicated logins from a one-per-line file."""
    lines = Path(path).read_text().splitlines()
    return sorted({line.strip() for line in lines if line.strip()}, key=str.lower)


def main():
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--dir", default="assets/contributors", help="contributor directory (holds the list plus svg/ and avatars/)")
    parser.add_argument("--txt", default="contributors.txt", help="login-list filename within --dir")
    parser.add_argument("--refresh", action="store_true", help="re-download avatars instead of using the cache")
    args = parser.parse_args()

    directory = Path(args.dir)
    names = read_logins(directory / args.txt)
    svg_dir = directory / "svg"
    write_svgs(names, svg_dir, directory / "avatars", refresh=args.refresh)
    print(f"wrote {len(names)} SVGs to {svg_dir}/", file=sys.stderr)


if __name__ == "__main__":
    main()
