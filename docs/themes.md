# Making a theme

A theme is a folder in the launcher's `themes` folder (Settings › Appearance › Open themes folder) holding one file, `theme.toml`, and optionally one picture. The folder's name is the theme's id: 1 to 48 lowercase letters, digits or hyphens, starting with a letter or digit. A folder called `tidepool` is chosen as `local:tidepool`.

The launcher reads the folder when it starts and when you press Reload. It never changes your files.

```toml
version = 1
name = "Tidepool"
background = "shore.webp" # optional

[palette]
card = "#1A3135"
chrome = "#11262B"
text = "#F0F8F4"
muted = "#C3D7D2"
background = "#102328"
accent = "#8EDBCB"
```

A copy to start from is in [`themes/tidepool`](themes/tidepool/theme.toml).

| Key | What it is |
|---|---|
| `version` | Required. The number `1`. |
| `name` | Required. 1 to 64 characters of plain text on one line. |
| `background` | Optional. The name of one PNG, JPEG or still WebP picture in the same folder, shown in the Home banner. Without it, the pages show Dawn's scene. |
| `look` | Optional. `"dark"` (the default) or `"light"`: whether the scene and glass around your colours are Sunrise's night or its day. |
| `[palette]` | Required, with at least one of the colours below. A colour you leave out is Dawn's. |
| `palette.background` | The page behind the scenery. |
| `palette.card` | The tint of the panels on a page. |
| `palette.chrome` | The tint of the sidebar and the top bar. |
| `palette.text`, `palette.muted` | Text, and quieter text. |
| `palette.accent` | The accent: Play's fill, marks and progress. |
| `palette.line` | Dividers and outlines. Left out, the text's colour. |
| `palette.success`, `palette.danger` | The colours of done and of wrong. |

A colour is written `"#RRGGBB"`.

What a theme can't set: opacity, blur, fonts, sizes, corners, or the label on Play. Play is a solid fill with a white label in every theme; under a pale accent only Play's fill is darkened until white reads on it. Hover, selection, field, edge, focus and status colours are worked out from your colours, and each ink is adjusted if it would not read on its surface.

## When a theme is refused

A folder the launcher can't use is listed under the themes with the reason, and the other themes still work. A file is refused when:

- it is over 16 KiB, isn't UTF-8 TOML, or writes a key twice;
- it has a key not in the table above (`opacity`, `accent_on`, a `[glass]` table…);
- `version` isn't `1`, `name` is missing or not plain one-line text, or a colour isn't `"#RRGGBB"`;
- `background` is anything but one file name: no folders, `..`, drive letters or addresses;
- the picture is missing, isn't a PNG, JPEG or still WebP by its contents, is over 40 MB, is under 64 or over 8192 pixels on a side, or is over 16 megapixels;
- the folder, `theme.toml` or the picture is a link, or `theme.toml` or the picture is not a regular file;
- `background`, `card` and `chrome` are so far apart that neither black nor white text reads on all of them. Make them all dark or all light.

If the theme you chose stops loading, the launcher shows Dawn and keeps your choice: repair the file and press Reload.

## From the command line

```
sunrise-launcher settings theme local:tidepool
sunrise-launcher settings theme default          # Dawn
sunrise-launcher settings accent '#8edbcb'       # or a name, or a hue in degrees
sunrise-launcher settings accent theme           # back to the theme's own
sunrise-launcher settings background ~/shore.png # your own picture in the Home banner
sunrise-launcher settings background default     # back to the theme's
sunrise-launcher settings scenery off
sunrise-launcher settings reduced-graphics on    # keep the background, reduce effects and motion
```

**Reduced graphics** in Settings › Appearance leaves your chosen background in place. The
separate **Show background** switch controls whether you see it.

## Sharing themes

Settings › Appearance edits these same colours, keeps the themes you save in this folder, and turns any of them into a share code (`SUN1-…`) that another player pastes under Import. A code holds only names and colours, never a picture.
