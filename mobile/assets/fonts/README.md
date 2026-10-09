# Fonts

The cabinet's three faces (`server/webui/vendor/fonts/`), as static TTF because
Flutter does not read `woff2`. All are SIL Open Font License 1.1; each licence is
next to its files.

| File | Source |
|---|---|
| `Inter-{Regular,Medium,SemiBold}.ttf` | [rsms/inter](https://github.com/rsms/inter) release v4.1, `extras/ttf/` |
| `SpaceGrotesk-{Medium,Bold}.ttf` | [floriankarsten/space-grotesk](https://github.com/floriankarsten/space-grotesk) `master`, `fonts/ttf/static/` (there is no SemiBold static; 600 resolves to Bold) |
| `JetBrainsMono-{Regular,Medium}.ttf` | [JetBrains/JetBrainsMono](https://github.com/JetBrains/JetBrainsMono) tag v2.304, `fonts/ttf/` |

They are bundled rather than fetched at run time, for the same reason the
cabinet vendors its own: a school network may have no route to Google.
