#!/usr/bin/env bash
# Genera el .deb de swarm en target/debian/.
# Requiere: cargo-deb (cargo install cargo-deb), libgtk-4-dev y libvte-2.91-gtk4-dev.
set -euo pipefail
cd "$(dirname "$0")/.."

# cargo-deb solo compila el paquete `swarm`; los otros binarios los compilamos antes.
cargo build --release --workspace --bins
cargo deb -p swarm --no-build "$@"

deb=$(ls -t target/debian/swarm_*.deb | head -1)
echo
echo "Listo: $deb"
echo "Instalar con: sudo apt install ./$deb"
