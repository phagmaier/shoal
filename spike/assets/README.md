# Bundled font: provenance and licenses

`SymbolsNerdFontMono-Regular.ttf` is the unmodified file from the Nerd Fonts **v3.5.1** release archive
`NerdFontsSymbolsOnly.zip`:

| What | Value |
| --- | --- |
| Download | https://github.com/ryanoasis/nerd-fonts/releases/download/v3.5.1/NerdFontsSymbolsOnly.zip |
| Archive SHA-256 | `fdca3682534f6f65e1ccb2345b0362ccf67d9b8eca7c8025330946e93e2473bc` |
| Font SHA-256 | `fe471e538392f51910faab985fa8e192a39dd3426125edd15b71b3680df0e749` |
| Verified | 2026-10-25: downloaded again and compared byte for byte with the committed file |

The spike embeds it with `include_bytes!` in `gui/src/main.rs`.

## The font is not under one license

The archive's own `LICENSE` is MIT ([licenses/nerd-fonts-LICENSE](licenses/nerd-fonts-LICENSE)), and Nerd Fonts lists the
Symbols Only font as MIT. But the glyphs come from many icon sets with their own licenses. The archive's README
lists them ([licenses/nerd-fonts-symbols-only-README.md](licenses/nerd-fonts-symbols-only-README.md)):

| Icon set | Upstream license | License text kept here |
| --- | --- | --- |
| Codicons | CC BY 4.0 | [codicons-LICENSE.txt](licenses/codicons-LICENSE.txt) (full CC BY 4.0) |
| Devicons | MIT | None upstream at v3.5.1 |
| extraglyphs (Hack) | MIT | None upstream at v3.5.1 |
| Font Awesome 6.5.1 | CC BY 4.0 (icons); OFL 1.1, RFN "Font Awesome" (font files) | [font-awesome-LICENSE.txt](licenses/font-awesome-LICENSE.txt) |
| Font Awesome Extension | MIT | None upstream at v3.5.1 |
| Font Logos | "unlicensed" | None: no license grants redistribution |
| MaterialDesign | Apache 2.0 | [materialdesign-LICENSE](licenses/materialdesign-LICENSE), [Apache-2.0.txt](licenses/Apache-2.0.txt) |
| Octicons | MIT | [octicons-LICENSE](licenses/octicons-LICENSE) |
| Seti and original | MIT | None upstream at v3.5.1 |
| Pomicons | OFL 1.1, RFN "Pomicons" | [pomicons-LICENSE](licenses/pomicons-LICENSE) |
| Powerline Extra | MIT | [powerline-extra-LICENSE](licenses/powerline-extra-LICENSE) |
| Powerline Symbols | MIT | [powerline-symbols-LICENSE.txt](licenses/powerline-symbols-LICENSE.txt) |
| Power Symbols IEC | MIT | None upstream at v3.5.1 |
| Weather Icons | OFL 1.1 | [weather-icons-OFL.txt](licenses/weather-icons-OFL.txt) (upstream copy still has template placeholders) |

The per-set files are copied unchanged from `src/glyphs/` in the nerd-fonts repository at tag `v3.5.1`, as is its
[license audit](licenses/nerd-fonts-license-audit.md). `Apache-2.0.txt` comes from apache.org because the
MaterialDesign notice only links to it.

## Status

This attribution covers the throwaway spike branch, which is never merged or shipped. Before Shoal ships the font,
someone has to decide what to do about the Font Logos glyphs (no license) and the MIT sets that have no license
text. The options are to subset them out or to obtain terms. Shoal's packaging must also install the license
texts. Do not describe this font as uniformly MIT or OFL.
