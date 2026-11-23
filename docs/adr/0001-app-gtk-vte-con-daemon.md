# App GTK con terminales VTE y un daemon propio para la persistencia

Swarm es una app de escritorio GTK que embebe terminales VTE (el motor de GNOME Terminal), y los procesos viven en un daemon propio que sobrevive al cierre de la ventana. Lo elegimos para que se sienta como la terminal nativa del usuario. El usuario no usa tmux, y envolver tmux lo obligaría a vivir con sus atajos y vería las señales de la terminal de forma indirecta. Los procesos sobreviven mientras la máquina esté prendida; tras un reinicio solo se restaura la Estructura.

## Considered Options

- **Wrapper de tmux**: daba la persistencia gratis, pero imponía tmux y hacía indirecta la lectura de BEL/OSC.
- **Electron/Tauri + xterm.js**: más portable, pero se siente menos nativo y la portabilidad no hace falta (es solo Linux local).

## Consequences

- El daemon es dueño de cada pseudo-terminal y la ventana solo muestra lo que le llega. Al reconectar se repite un buffer acotado de salida cruda y se fuerza un redimensionado para que los programas a pantalla completa se redibujen. No emulamos la pantalla en el daemon, así que puede quedar algo de basura visual.
- La app arranca el daemon si no está corriendo, y el daemon sigue vivo al cerrar la ventana.
