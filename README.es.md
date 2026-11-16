# OpenDoc CLI

[English](README.md)

[![Release](https://img.shields.io/github/v/release/daidarzzz/opendoc-cli)](https://github.com/daidarzzz/opendoc-cli/releases)
[![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue)](LICENSE)

Búsqueda y apertura de docsets de **Dash/Zeal** fuera de línea, desde la
terminal. Comparte con la app [OpenDoc](https://github.com/daidarzzz/opendoc)
el mismo núcleo ([`opendoc-core`](https://github.com/daidarzzz/opendoc-core)):
escaneo, índices SQLite (`searchIndex` y Core Data), cachés tarix y
búsqueda difusa. Sin conexión, sin telemetría y sin dependencia de los
servidores de Kapeli.

Puedes invocar la herramienta tanto con su nombre completo (`opendoc-cli`) como con el alias corto (`odc`).

> Los ejemplos muestran la salida en color (paleta Catppuccin Mocha); `anstream`
> la desactiva automáticamente si se detecta la variable `NO_COLOR` o al redirigir
> (por ejemplo `| head` o `> salida.txt`).

## Instalación

### En una sola línea (Zero-manual download)

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/daidarzzz/opendoc-cli/master/install.ps1 | iex
```

Descarga la última release, extrae `odc.exe` y `opendoc-cli.exe` en `$HOME\.local\bin` y añade la ruta persistentemente al PATH de usuario si aún no está presente.

**Linux y macOS:**

```bash
curl -fsSL https://raw.githubusercontent.com/daidarzzz/opendoc-cli/master/install.sh | sh
```

Detecta tu sistema operativo y arquitectura (`x86_64` o `aarch64` / Apple Silicon), descarga el paquete comprimido e instala `odc` y `opendoc-cli` en `~/.local/bin`.

---

### Desde el código fuente (Cargo)

```bash
cargo install --git https://github.com/daidarzzz/opendoc-cli
```

Esto compilará e instalará automáticamente ambos binarios (`opendoc-cli` y `odc`) en tu carpeta de binarios de Cargo (`~/.cargo/bin`).

---

### Binarios ya compilados (manual)

Descarga el archivo correspondiente a tu plataforma desde la
[página de releases](https://github.com/daidarzzz/opendoc-cli/releases) y
descomprímelo. Los paquetes incluyen tanto `opendoc-cli` como `odc`, optimizados por tamaño:

| Plataforma | Archivo |
| ---------- | ------- |
| Windows x86_64 | `opendoc-cli-<versión>-x86_64-pc-windows-msvc.zip` |
| Linux x86_64 | `opendoc-cli-<versión>-x86_64-unknown-linux-gnu.tar.gz` |
| macOS (Apple Silicon) | `opendoc-cli-<versión>-aarch64-apple-darwin.tar.gz` |
| macOS (Intel) | `opendoc-cli-<versión>-x86_64-apple-darwin.tar.gz` |

## Primeros pasos

```bash
# 1. Lanza la interfaz interactiva de pantalla completa (TUI)
odc

# 2. Lee documentación directamente en la terminal sin abrir el navegador
odc show "Vec::push"
odc cat "bash:echo"

# 3. Muestra solo ejemplos de código o copia al portapapeles
odc show "Vec::push" --code
odc show "std::vector" --copy

# 4. Busca con badges compactos de tipo
odc search grid --limit 5

# 5. Sintaxis directa docset:query
odc search css:grid
odc open java:string
```

## Gestor de docsets

```bash
odc docsets catalog python
odc docsets install <ID>
odc docsets refresh
odc docsets update <ID>
odc docsets uninstall <ID>
```

Usa el ID que aparece entre corchetes en el catálogo. La primera consulta descarga el catálogo si hace falta. `odc docsets update <ID> --force` permite reinstalar y `odc docsets uninstall <ID> --yes` omite la confirmación en scripts.

En la interfaz de terminal, **F4** abre el catálogo. Escribe para filtrar y usa **Enter** para instalar, actualizar o reinstalar. **Supr** pide confirmación para desinstalar, **F5** actualiza el catálogo, **F6** cambia los feeds y **Esc** vuelve al lector.

DevDocs está disponible de serie. Con `odc docsets feeds set owner/repo` puedes añadir feeds de GitHub. `feeds reset` vuelve a usar la configuración de OpenDoc. El CLI comparte la carpeta de docsets con OpenDoc por defecto y guarda aparte sus cambios de configuración y sus descargas del catálogo.

## Actualizar el CLI

Comprueba si hay una nueva versión con `odc update --check`. Ejecuta `odc update` para actualizar los dos binarios de una instalación oficial. Conserva el fichero `opendoc-cli.release` junto a los ejecutables si los instalas a mano. Para actualizar desde v0.2.0, vuelve a ejecutar el instalador original o descarga un paquete nuevo. Si instalaste con Cargo, vuelve a ejecutar `cargo install`.

## Comandos

### `tui` (o simplemente `odc` sin argumentos)

Lector de documentación de pantalla completa (Ratatui, paleta Catppuccin Mocha):

- **Búsqueda difusa en vivo** con la letra coincidente resaltada y la sintaxis `rust:Vec::push`, `fn:map`, `py:fn:dumps` (chips de docset y tipo visibles en la barra).
- **Lector real**, no un volcado de texto: encabezados, listas, tablas, citas, código resaltado con `syntect` y enlaces. Re-maqueta al redimensionar, con barra de scroll y porcentaje.
- **Enlaces navegables**: `Tab` selecciona el siguiente enlace, `Enter` lo sigue dentro del propio docset y `⌫` vuelve atrás (historial).
- **Buscar en la página** (`/`, luego `n`/`N`): resaltado de todas las coincidencias (mayúsculas inteligentes) y contador.
- **Índice de la página** (`O`, `gO` o `Ctrl+O`): panel con los encabezados para saltar a «Examples» o «Panics».
- **Relacionados** (`r`): otros símbolos de la misma página o del mismo tipo (los métodos de `Vec`), sin salir del lector.
- **Favoritos y recientes** (`m` marca ★; `H`, `Ctrl+H` o `F2` los abren): se guardan en `cli-state.json` (directorio de datos de OpenDoc, o la ruta de `ODC_STATE`) y se abren sin buscar; `Supr` olvida una entrada.
- **Selectores** de docset (`Ctrl+D`) y de tipo de símbolo (`Ctrl+T`) con filtro incremental.
- **Ratón**: rueda para desplazar, clic para elegir resultado y **selección de texto propia** en el lector: arrastra para seleccionar (se copia al soltar), doble clic = palabra, triple clic = línea. Arrastrar fuera del panel hace autoscroll.
- **Contexto de proyecto**: dentro de un directorio con `Cargo.toml`, `package.json`, `go.mod`, `pyproject.toml`… la TUI arranca filtrada por el docset del lenguaje (`Esc` limpia el filtro; `--no-context` lo desactiva).
- Adaptable: en terminales estrechas (< 90 columnas) muestra una sola columna.
- La vista previa se carga solo cuando el teclado queda en reposo, así que mantener pulsada una tecla nunca se atasca.

| Búsqueda | |
| --- | --- |
| `↑↓` `Ctrl+N/P` | mover selección |
| `Enter` `Tab` `→` | leer el símbolo |
| `Ctrl+D` / `Ctrl+T` | elegir docset / tipo |
| `Shift+Tab` | ciclar docset |
| `Ctrl+W` `Ctrl+U` | borrar palabra / línea |
| `Ctrl+Y` / `Ctrl+B` | copiar ejemplo / abrir en el navegador |
| `Ctrl+H` `F2` `Alt+H` | favoritos y recientes |
| `Ctrl+F` / `Ctrl+O` / `Ctrl+R` | marcar ★ / índice / relacionados |

| Lector | |
| --- | --- |
| `j` `k` `Space` `b` `Ctrl+D/U` `gg` `G` | desplazamiento |
| `/` `n` `N` | buscar en la página / siguiente / anterior |
| `O` `gO` | índice de secciones |
| `r` | símbolos relacionados |
| `m` `H` | marcar favorito / favoritos y recientes |
| `[` `]` | sección anterior / siguiente |
| `Tab` `Shift+Tab` `Enter` `⌫` | enlaces e historial |
| `y` `Y` `s` | copiar ejemplo / todo / firma |
| `o` | abrir la página en el navegador |
| `d` `t` | elegir docset / tipo |
| `/` `i` | volver a la búsqueda |
| `?` `F1` | ayuda · `q` / `Ctrl+C` salir |

El portapapeles usa `wl-copy`, `xclip` o `xsel` (y OSC 52 como último recurso, válido por SSH); en macOS y Windows, el del sistema.

### `show` (alias `cat`)

Renderiza la documentación directamente en la terminal con el mismo motor que la TUI:

- Extrae la sección exacta del símbolo (`#method.push`…) **incluida su descripción**, aunque el HTML la coloque fuera del ancla.
- Cabecera con tipo, docset y versión (`Stable since…`), firma resaltada, listas, tablas y código con TrueColor.
- Paginador automático (`less -R`) si no cabe en pantalla; se degrada a texto plano al redirigir.

```console
$ odc show "rust:Vec::push"
 fn  std::vec::Vec::push
rust  ·  Stable since Rust version 1.0.0, const unstable

 pub fn push(&mut self, value: T)
────────────────────────────────────────────────────────────

Appends an element to the back of a collection.
…
```

Flags de `show`: `--code` (solo firma y ejemplos), `--copy`, `--raw` (texto plano), `--json` (contrato `version: 1`) y `--width <COLS>`.

### Sintaxis de consulta

`docset:texto`, `tipo:texto` o ambos (`rust:fn:push`). Los docsets admiten id, nombre y alias (`py`, `js`, `ts`, `rs`, `sh`). `std::vector` no se confunde con un prefijo. Los docsets de DevDocs guardan una *categoría* en el campo `kind`; el CLI infiere el tipo real (`fn`, `struct`, `trait`, `macro`…) desde la ruta.

Las búsquedas de `search`, `show` y `open` priorizan automáticamente el docset del proyecto actual (sin descartar el resto). Desactívalo con `--no-context`.

### `completions`

```bash
odc completions zsh > ~/.zfunc/_odc      # bash, zsh, fish, powershell, elvish
```

### `list`

Lista los docsets cargados y los tarix todavía pendientes de extraer.

```console
$ odc list
docsets (2)
  css      CSS      -          1249 entries
  swift    Swift    -          9467 entries

pending tarix (2)
  c        C++      (not extracted: open it once in OpenDoc)
  python-3 Python_3 (not extracted: open it once in OpenDoc)
```

### `search`

Búsqueda difusa por nombre de entrada en todos los docsets o filtrando por uno. Presenta badges compactos alineados (`[fn]`, `[cls]`, `[prop]`, `[guide]`, `[var]`, etc.).

```console
$ odc search grid --limit 5
search "grid"
NAME              KIND     DOCSET
grid              [prop]   css
grid-area         [prop]   css
grid-auto-columns [prop]   css
grid-auto-flow    [prop]   css
grid-auto-rows    [prop]   css
5 result(s) · 10716 entries indexed · use `open` to launch one
```

#### Sintaxis directa `docset:query`

Puedes prefijar cualquier búsqueda con el id o nombre del docset seguido de dos puntos:

```console
$ odc search css:color --limit 5
search "css:color"
NAME        KIND     DOCSET
color       [fn]     css
color       [prop]   css
color       [type]   css
color-mix   [fn]     css
color-gamut [prop]   css
5 result(s) · 10716 entries indexed · use `open` to launch one
```

También puedes usar la flag explícita `--docset` (acepta id, nombre o plataforma):

```console
$ odc search color --docset css --limit 5
```

Salida para máquinas con `--json`:

```console
$ odc search grid --json --limit 2
[
  {
    "docset_id": "css",
    "name": "grid",
    "kind": "Property",
    "path": "developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/grid.html",
    "score": 114
  },
  {
    "docset_id": "css",
    "name": "grid-area",
    "kind": "Property",
    "path": "developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/grid-area.html",
    "score": 114
  }
]
```

### `open`

Abre la documentación correspondiente en el visor o navegador del sistema:

1. **1 única coincidencia:** La abre directamente sin confirmación ni pausas.
2. **Múltiples coincidencias en TTY interactiva:** Muestra un selector con flechas ↑/↓ (o `j`/`k`) para elegir cuál abrir, mostrando su tipo, nombre y docset. Si pulsas `Esc` o `Ctrl+C`, cancela limpiamente con código 0.
3. **Modo no interactivo o scripts:** Con `--non-interactive`, `--exact` o cuando la salida estándar no es una terminal interactiva (por ejemplo tuberías), abre directamente el mejor resultado.

```console
# Abre directamente con sintaxis rápida
$ odc open css:grid
[prop] grid
docset: css  opendoc://css/developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/grid.html
file:   /home/you/docsets/CSS.docset/Contents/Resources/Documents/developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/grid.html

# Múltiples coincidencias (menú interactivo):
$ odc open vector
? Select document to open:
> [cls]    Vector                               [java]
  [struct] std::vec::Vec                        [rust]
  [fn]     vector                               [cpp]
[↑↓ to move, enter to select, type to filter]
```

## Flags

Los flags globales (`--docsets`, `--tarix-cache`) se pueden escribir antes
o después del subcomando.

| Flag | Comandos | Descripción |
| ---- | -------- | ----------- |
| `--docsets <DIR>` | todos | Carpeta que contiene los `*.docset`. Por defecto, los ajustes de OpenDoc. |
| `--tarix-cache <DIR>` | todos | Base de la caché tarix. Por defecto, los datos de OpenDoc. |
| `--docset <DOCSET>` | `search`, `open` | Restringe la búsqueda a un docset (id, nombre o familia). |
| `--limit <N>` | `search` | Número máximo de resultados. Por defecto `20`, tope `500`. |
| `--no-context` | todos | No priorizar el docset del proyecto actual. |
| `--json` | `search`, `list`, `show` | Emite JSON en lugar de la tabla con color. |
| `--non-interactive` | `open` | Desactiva el selector interactivo si hay múltiples resultados y abre el primero. |
| `--exact` | `open` | Abre la mejor coincidencia sin interactividad. |
| `-h`, `--help` | todos | Ayuda del comando con paleta de colores coherente. |
| `-V`, `--version` | — | Versión del binario. |

## Códigos de salida

| Código | Significado |
| -----: | ----------- |
| `0` | Éxito. Incluye búsquedas sin resultados y cancelaciones voluntarias en el selector interactivo (`Esc` / `Ctrl+C`). |
| `1` | Error de ejecución: carpeta ausente, docset desconocido, sin extraer o fallo al abrir. |
| `2` | Error de uso: argumentos inválidos (los gestiona `clap`). |

## Dónde busca los docsets

La carpeta de docsets se resuelve en este orden, de más explícito a menos:

1. El flag `--docsets DIR`.
2. La variable de entorno `OPENDOC_DOCSETS`.
3. El campo `docsets_dir` de los ajustes de la app OpenDoc.
4. `<datos-del-sistema>/com.opendoc.app/docsets`.

La caché tarix sigue la misma precedencia con `--tarix-cache` /
`OPENDOC_TARIX_CACHE` y, por defecto,
`<datos-del-sistema>/com.opendoc.app/tarix-cache`.

El CLI **nunca extrae** los tarix: reutiliza las cachés que la app ya haya
creado. Si ejecutas el CLI antes de abrir un docset en OpenDoc, sus
documentos no estarán disponibles y `list` lo marcará como
`(not extracted)`. Ábrelo una vez en la app y ya quedará disponible para el
CLI.

## Compilar y probar

```bash
cargo build --release --bins   # binarios: target/release/{opendoc-cli, odc}
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Este repositorio es **independiente** de la app: no comparte perfiles de
Cargo, `target/`, lockfiles ni configuración. Consume el núcleo por
etiqueta:

```toml
opendoc-core = { git = "https://github.com/daidarzzz/opendoc-core", tag = "v0.2.0" }
```

## Licencia

GPL-3.0-or-later. Ver [`LICENSE`](LICENSE).
