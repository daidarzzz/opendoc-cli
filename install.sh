#!/bin/sh
# Script de instalación desatendida para OpenDoc CLI en Linux / macOS
# Uso en 1 línea:
#   curl -fsSL https://raw.githubusercontent.com/daidarzzz/opendoc-cli/master/install.sh | sh

set -eu

REPO="daidarzzz/opendoc-cli"
INSTALL_DIR="$HOME/.local/bin"

echo "Instalando OpenDoc CLI (odc)..."

# 1. Detectar sistema y arquitectura
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$OS" in
  linux)
    case "$ARCH" in
      x86_64) TARGET="x86_64-unknown-linux-gnu" ;;
      *) echo "Error: arquitectura no soportada en Linux: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  darwin)
    case "$ARCH" in
      arm64|aarch64) TARGET="aarch64-apple-darwin" ;;
      x86_64) TARGET="x86_64-apple-darwin" ;;
      *) echo "Error: arquitectura no soportada en macOS: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "Error: sistema operativo no soportado: $OS" >&2
    exit 1
    ;;
esac

# 2. Obtener URL de la release
LATEST_JSON="$(curl -fsSL -H "User-Agent: opendoc-cli-installer" "https://api.github.com/repos/${REPO}/releases/latest")"
DOWNLOAD_URL="$(echo "$LATEST_JSON" | grep -o 'https://[^"]*' | grep "${TARGET}\.tar\.gz" | head -n 1)"

if [ -z "$DOWNLOAD_URL" ]; then
  echo "Error: no se encontró un archivo para ${TARGET} en la última release de ${REPO}." >&2
  exit 1
fi

# 3. Descargar y descomprimir en directorio temporal
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

echo "Descargando binario (${TARGET})..."
curl -fsSL "$DOWNLOAD_URL" | tar -xz -C "$TMPDIR"

mkdir -p "$INSTALL_DIR"

for bin in opendoc-cli odc; do
  if [ -f "$TMPDIR/$bin" ]; then
    install -m 0755 "$TMPDIR/$bin" "$INSTALL_DIR/$bin" 2>/dev/null || cp "$TMPDIR/$bin" "$INSTALL_DIR/$bin"
    chmod 0755 "$INSTALL_DIR/$bin"
  fi
done

echo ""
echo "OpenDoc CLI se ha instalado correctamente en $INSTALL_DIR"

# 4. Comprobar si INSTALL_DIR está en PATH
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo ""
    echo "Aviso: $INSTALL_DIR no está en tu \$PATH actual."
    echo "Para poder invocar 'odc' y 'opendoc-cli', añade esta línea a tu ~/.bashrc o ~/.zshrc:"
    echo '  export PATH="$HOME/.local/bin:$PATH"'
    ;;
esac
