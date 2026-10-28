# Notices

## BlueBubbles

The app icon (`assets/icons/bluebubbles-*.png`) is resized from
`assets/icon/icon.png` in [bluebubbles-app](https://github.com/BlueBubblesApp/bluebubbles-app),
Apache License 2.0. This project is an independent client for the
BlueBubbles Server and is not affiliated with the BlueBubbles project.

## Dictionary

The en_US Hunspell dictionary (`assets/dictionaries`) is version
2020.12.07, made from [SCOWL](http://wordlist.aspell.net) by Kevin
Atkinson and others. Its permissive license is in
`assets/dictionaries/en_US.license`, which the .deb installs as
`usr/share/doc/bluebubbles-gpui/en_US.license`.

## Fonts

- Inter 4.1 (`assets/fonts/inter`), SIL Open Font License 1.1, by The
  Inter Project Authors. Modified: static instances taken from `Inter.ttc`
  with U+2600, U+2665, U+26A0 and U+2764 removed from the character map.
- Noto Color Emoji 2.051 (`assets/fonts/emoji`), SIL Open Font License
  1.1, by Google. Modified: renamed "BlueBubbles Color Emoji" and U+006D
  mapped to the space glyph.

`scripts/edit-fonts.py` makes both modifications.
