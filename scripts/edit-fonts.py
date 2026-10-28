"""Prepares the bundled fonts for GPUI on Linux (see crates/ui/src/fonts.rs).

Usage, from the repository root:

    nix shell --impure --expr 'with import (builtins.getFlake "nixpkgs") {}; python3.withPackages (p: [p.fonttools])' --command python3 -I \
      scripts/edit-fonts.py \
      $(nix build --no-link --print-out-paths nixpkgs#noto-fonts-color-emoji)/share/fonts/noto/NotoColorEmoji.ttf \
      $(nix build --no-link --print-out-paths nixpkgs#inter)/share/fonts/truetype/Inter.ttc \
      assets/fonts
"""
import sys
from fontTools.ttLib import TTFont
from fontTools.ttLib.ttCollection import TTCollection

emoji_src, ttc_src, out = sys.argv[1:4]

# GPUI drops a fallback family without a glyph for `m`, so map it to the
# space glyph (ASCII never falls back, so it is never drawn). GPUI draws
# colour only for the PostScript name NotoColorEmoji, so that stays; the
# family is renamed so a system copy is left alone.
emoji = TTFont(emoji_src)
space = emoji.getBestCmap()[0x20]
for table in emoji["cmap"].tables:
    if table.isUnicode() and table.format != 14:
        table.cmap[0x6D] = space
for name in emoji["name"].names:
    if name.nameID in (1, 3, 4, 16):
        name.string = "BlueBubbles Color Emoji"
emoji.save(f"{out}/emoji/BlueBubblesColorEmoji.ttf")

# GPUI keeps a character in the primary font when it covers it; drop these
# from Inter so they draw as colour emoji, as Messages draws them.
KEEP = {"Inter-Regular", "Inter-Italic", "Inter-Medium", "Inter-SemiBold", "Inter-Bold"}
for font in TTCollection(ttc_src).fonts:
    name = font["name"].getDebugName(6)
    if name not in KEEP:
        continue
    for table in font["cmap"].tables:
        for cp in (0x2600, 0x2665, 0x26A0, 0x2764):
            table.cmap.pop(cp, None)
    font.save(f"{out}/inter/{name}.ttf")
