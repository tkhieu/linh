# Third-party notices

The bundled UI fonts are licensed under the SIL Open Font License 1.1. Their
copyright notices and full license texts are shipped with the app in the
frontend assets:

| Font | License text |
| --- | --- |
| IBM Plex Sans | [ibm-plex-sans-OFL.txt](ui/public/licenses/ibm-plex-sans-OFL.txt) |
| IBM Plex Mono | [ibm-plex-mono-OFL.txt](ui/public/licenses/ibm-plex-mono-OFL.txt) |
| Martian Mono | [martian-mono-OFL.txt](ui/public/licenses/martian-mono-OFL.txt) |

These files come from the corresponding `@fontsource` packages. Vite copies
`ui/public/licenses/` into the frontend distribution included in desktop
packages.

Rust and npm dependencies retain their own licenses, predominantly MIT or
Apache-2.0, with some MPL-2.0 dependencies. The license of Linh does not
replace the licenses of those dependencies.
