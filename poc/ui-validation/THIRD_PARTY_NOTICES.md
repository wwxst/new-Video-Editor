# Third-party notices for the Slint POC

## Slint 1.19.1

`slint 1.19.1` declares:

```text
GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0
```

The complete Royalty-free 2.0 and Software 3.0 texts are in `THIRD_PARTY_LICENSES/`.

For a closed-source desktop application the **Slint Royalty-free Desktop, Mobile, and Web
Applications License 2.0** is the relevant option. It requires either:

1. displaying the `AboutSlint` widget in an About screen/dialog reachable from the top-level menu
   (or on the splash screen), or
2. displaying the “Made with Slint” badge on a public download page.

The POC uses option 1: the INSPECTOR panel's “关于 Slint” button opens a dialog containing
`AboutSlint` (see `reports/screenshots/slint-about.png`).

The license does not permit distributing Slint alone, embedding it in an “Embedded System” as
defined by the license, or exposing Slint's APIs. It does not require releasing the application's
source.

This notice is an engineering summary, not legal advice. Have counsel confirm the chosen license
and attribution wording before shipping.

## Embedded font

`ui/fonts/ArclineCJK.ttf` is the repository's `assets/fonts/NotoSansSC-Arcline.ttf`
(Noto Sans SC subset, SIL Open Font License; see `assets/fonts/OFL.txt`).
