# opendoc-cli / odc

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
descomprímelo. Los paquetes incluyen tanto `opendoc-cli` como `odc`, optimizados por tamaño (≈1,8 MB):

| Plataforma | Archivo |
| ---------- | ------- |
| Windows x86_64 | `opendoc-cli-<versión>-x86_64-pc-windows-msvc.zip` |
| Linux x86_64 | `opendoc-cli-<versión>-x86_64-unknown-linux-gnu.tar.gz` |
| macOS (Apple Silicon) | `opendoc-cli-<versión>-aarch64-apple-darwin.tar.gz` |
| macOS (Intel) | `opendoc-cli-<versión>-x86_64-apple-darwin.tar.gz` |

## Primeros pasos

```bash
# 1. ¿Qué docsets hay instalados?
odc list

# 2. Busca con badges compactos de tipo
odc search grid --limit 5

# 3. Sintaxis directa docset:query
odc search css:grid
odc open java:string

# 4. Abre la mejor coincidencia o elige con el selector interactivo
odc open std::vector
```

## Comandos

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
| `--json` | `search`, `list` | Emite JSON en lugar de la tabla con color. |
| `--non-interactive` | `open` | Desactiva el selector interactivo si hay múltiples resultados y abre el primero. |
| `--exact` | `open` | Abre la mejor coincidencia sin interactividad. |
| `-h`, `--help` | todos | Ayuda del comando con paleta de colores coherente. |
| `-V`, `--version` | — | Versión del binario (`0.1.2`). |

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
opendoc-core = { git = "https://github.com/daidarzzz/opendoc-core", tag = "v0.1.1" }
```

## Licencia

GPL-3.0-or-later. Ver [`LICENSE`](LICENSE).
