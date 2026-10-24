# opendoc-cli

Búsqueda y apertura de docsets de **Dash/Zeal** fuera de línea desde la
terminal. Comparte con la app [OpenDoc](https://github.com/daidarzzz/opendoc)
el mismo núcleo ([`opendoc-core`](https://github.com/daidarzzz/opendoc-core)):
escaneo, índices SQLite (`searchIndex` y Core Data), cachés tarix y
búsqueda difusa.

Este repositorio es **independiente** de la app: no comparte perfiles de
Cargo, `target/`, lockfiles ni configuración. Consume el núcleo por
etiqueta:

```toml
opendoc-core = { git = "https://github.com/daidarzzz/opendoc-core", tag = "v0.1.1" }
```

## Compilar

```bash
cargo build --release
# binario: target/release/opendoc-cli
```

## Uso

```
opendoc-cli [--docsets DIR] [--tarix-cache DIR] <COMANDO>
```

- `search <QUERY>... [--docset ID] [--limit N] [--json]`
- `open <QUERY>... [--docset ID]`
- `list [--json]`

```bash
opendoc-cli list
opendoc-cli search grid --docset css
opendoc-cli search "vector push_back" --limit 5
opendoc-cli open std::vector
opendoc-cli list --json | jq .
```

### Dónde busca los docsets

En este orden:

1. `--docsets DIR` / `--tarix-cache DIR`.
2. Variables `OPENDOC_DOCSETS` / `OPENDOC_TARIX_CACHE`.
3. `docsets_dir` de los ajustes de la app.
4. `<datos>/com.opendoc.app/{docsets,tarix-cache}`.

La caché tarix **nunca se extrae** desde el CLI: se reutilizan las que la
app ya haya creado. Los tarix pendientes aparecen en `list` como
`(not extracted)`.

### Alias

El binario se llama `opendoc-cli` para no chocar con el ejecutable de la
app. Si quieres el nombre corto, añade un alias:

```bash
alias opendoc=opendoc-cli
```

## Códigos de salida

| Código | Significado |
| -----: | ----------- |
| `0` | Éxito (incluye búsquedas sin resultados) |
| `1` | Error de ejecución (carpeta ausente, docset desconocido, fallo al abrir) |
| `2` | Error de uso (argumentos de clap) |

## Tests y estilo

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## Licencia

GPL-3.0-or-later. Ver `LICENSE`.
