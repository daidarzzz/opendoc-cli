# opendoc-cli

[![Release](https://img.shields.io/github/v/release/daidarzzz/opendoc-cli)](https://github.com/daidarzzz/opendoc-cli/releases)
[![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue)](LICENSE)

Búsqueda y apertura de docsets de **Dash/Zeal** fuera de línea, desde la
terminal. Comparte con la app [OpenDoc](https://github.com/daidarzzz/opendoc)
el mismo núcleo ([`opendoc-core`](https://github.com/daidarzzz/opendoc-core)):
escaneo, índices SQLite (`searchIndex` y Core Data), cachés tarix y
búsqueda difusa. Sin conexión, sin telemetría y sin dependencia de los
servidores de Kapeli.

> Los ejemplos muestran la salida en color; `anstream` la desactiva
> automáticamente al redirigir (por ejemplo `| head` o `> salida.txt`).

## Instalación

### Binarios ya compilados

Descarga el archivo de tu plataforma desde la
[página de releases](https://github.com/daidarzzz/opendoc-cli/releases) y
descomprímelo. Los binarios de release se compilan con `opt-level = "z"`,
`lto = true` y `strip = true` (≈1,7 MB).

| Plataforma | Archivo |
| ---------- | ------- |
| Windows x86_64 | `opendoc-cli-<versión>-x86_64-pc-windows-msvc.zip` |
| Linux x86_64 | `opendoc-cli-<versión>-x86_64-unknown-linux-gnu.tar.gz` |
| macOS (Apple Silicon) | `opendoc-cli-<versión>-aarch64-apple-darwin.tar.gz` |
| macOS (Intel) | `opendoc-cli-<versión>-x86_64-apple-darwin.tar.gz` |

Linux y macOS:

```bash
tar -xzf opendoc-cli-*-x86_64-unknown-linux-gnu.tar.gz
sudo install -m 0755 opendoc-cli /usr/local/bin/opendoc-cli
```

### Desde el código fuente

```bash
cargo install --git https://github.com/daidarzzz/opendoc-cli
```

## Primeros pasos

```bash
# 1. ¿Qué docsets hay instalados?
opendoc-cli list

# 2. Busca un símbolo o propiedad y muestra los aciertos.
opendoc-cli search grid --limit 5

# 3. Abre la mejor coincidencia en el visor del sistema.
opendoc-cli open std::vector
```

## Comandos

### `list`

Lista los docsets cargados y los tarix todavía pendientes de extraer.

```console
$ opendoc-cli list
docsets (2)
  css      CSS      -          1249 entries
  swift    Swift    -          9467 entries

pending tarix (2)
  c        C++      (not extracted: open it once in OpenDoc)
  python-3 Python_3 (not extracted: open it once in OpenDoc)
```

### `search`

Búsqueda difusa por nombre de entrada, en todos los docsets o en uno solo.

```console
$ opendoc-cli search grid --limit 5
search "grid"
NAME              KIND     DOCSET
grid              Property css
grid-area         Property css
grid-auto-columns Property css
grid-auto-flow    Property css
grid-auto-rows    Property css
5 result(s) · 10716 entries indexed · use `open` to launch one
```

Restringe a un docset con `--docset` (acepta id, nombre o familia):

```console
$ opendoc-cli search color --docset css --limit 5
search "color"
NAME        KIND     DOCSET
color       Function css
color       Property css
color       Type     css
color-mix   Function css
color-gamut Property css
5 result(s) · 10716 entries indexed · use `open` to launch one
```

Salida para máquinas con `--json`:

```console
$ opendoc-cli search grid --json --limit 2
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

Resuelve la mejor coincidencia y abre la página con la aplicación por
defecto del sistema (`cmd /C start`, `open` o `xdg-open`).

```console
$ opendoc-cli open grid --docset css
Property grid
docset: css  opendoc://css/developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/grid.html
file:   /home/you/docsets/CSS.docset/Contents/Resources/Documents/developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/grid.html
```

Acepta las mismas opciones que `search` (`--docset`), sin `--limit` ni
`--json`: siempre abre un único resultado.

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
| `-h`, `--help` | todos | Ayuda del comando. |
| `-V`, `--version` | — | Versión del binario. |

## Códigos de salida

| Código | Significado |
| -----: | ----------- |
| `0` | Éxito. Incluye búsquedas sin resultados (no es un error). |
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

```bash
# Apunta a una carpeta concreta sin tocar los ajustes de la app.
opendoc-cli --docsets ~/docsets search grid

# O de forma persistente para la sesión.
export OPENDOC_DOCSETS=~/docsets
```

## Alias

El binario se llama `opendoc-cli` para no chocar con el ejecutable de la
app (que en muchos sistemas también es `opendoc`). Si quieres el nombre
corto:

```bash
alias opendoc=opendoc-cli
```

En PowerShell:

```powershell
Set-Alias opendoc opendoc-cli
```

## Compilar y probar

```bash
cargo build --release          # binario: target/release/opendoc-cli
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
