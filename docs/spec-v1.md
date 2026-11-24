# Swarm v1: Especificación

Swarm es una app de escritorio para Linux (GNOME) que mantiene muchas Terminales vivas, agrupadas por Proyecto, para correr agentes de desarrollo sin perder de vista en qué está cada una.

Vocabulario: ver [GLOSSARY.md](../GLOSSARY.md). Decisiones de arquitectura: ver [docs/adr/](./adr/).

## 1. Alcance

**Incluido en la v1**
- Proyectos y Terminales Sueltas.
- Estado por Terminal sacado de señales estándar de terminal.
- Notificaciones de escritorio.
- Persistencia de los procesos mientras la máquina esté prendida, y de la Estructura después de un reinicio.
- Solo máquina local.

**Fuera de la v1**
- Splits y tabs.
- Comandos predefinidos por Proyecto.
- Integraciones específicas por agente (hooks, parseo de salida). Ver ADR 0002.
- Máquinas remotas, SSH y devcontainers.
- Arrancar el daemon como servicio de systemd.
- Configurar el umbral de Quieta.
- Mover Terminales entre Proyectos.
- Varias ventanas.

## 2. Proyectos

- **P1.** Un Proyecto tiene un nombre y un directorio raíz.
- **P2.** Las Terminales nuevas de un Proyecto arrancan en su directorio raíz.
- **P3.** Borrar un Proyecto que tiene Terminales vivas pide confirmación y después cierra todas sus Terminales, matando sus procesos.
- **P4.** Las Terminales sin Proyecto pertenecen a **Sueltas**, que siempre existe y no se puede borrar.

## 3. Terminales

- **T1.** Una Terminal nueva abre el shell por defecto del usuario, en el directorio del Proyecto o en `~` si es Suelta. Swarm no lanza ningún otro comando.
- **T2.** Una Terminal se comporta como una terminal común: swarm no interpreta ni modifica lo que pasa adentro.
- **T3.** Una Terminal tiene un **Nombre** opcional, que pone el usuario y se puede editar.
- **T4.** El **Título** de una Terminal es el que el programa declara con OSC 0/2.
- **T5.** Una Terminal no se puede mover a otro Proyecto.
- **T6.** Cerrar una Terminal viva pide confirmación y después mata su proceso.
- **T7.** Cerrar una Terminal Terminada no pide confirmación.

## 4. Estado

Cada Terminal está en un solo estado. Las señales que lo determinan son:

| Estado | Cuándo entra | Cuándo sale |
|---|---|---|
| **Activa** | Hubo salida en los últimos 20 s | Pasan 20 s sin salida (va a Quieta) |
| **Quieta** | Pasan 20 s sin salida | Hay salida nueva (va a Activa) |
| **Pide atención** | El programa emite BEL o una notificación OSC 9 / OSC 777 | El usuario **Mira** la Terminal (vuelve a Activa o Quieta según la actividad) |
| **Terminada** | El proceso salió (se registra el código de salida) | Nunca: queda así hasta que se cierra la Terminal |

- **E1.** Pide atención es pegajoso: la salida nueva no lo despeja, solo lo despeja Mirar.
- **E2.** **Mirar** una Terminal es tenerla visible *con la ventana de swarm enfocada*. Si está visible pero la ventana no tiene foco, no cuenta como mirada.
- **E3.** Si un proceso que Pide atención termina, la Terminal pasa a Terminada.
- **E4.** El umbral de 20 s es fijo y no se puede configurar en la v1.
- **E5.** Una Terminada que todavía no se Miró desde que terminó cuenta como "Terminada sin mirar". Se usa en N1 y A1.

## 5. Notificaciones

- **N1.** Swarm manda una notificación de escritorio de GNOME cuando una Terminal pasa a **Pide atención** o a **Terminada**, salvo que el usuario la esté Mirando en ese momento.
- **N2.** La notificación muestra el Proyecto, el Nombre o el Título de la Terminal y el evento. Para una Terminada incluye el código de salida.
- **N3.** Hacer clic en la notificación enfoca swarm y muestra esa Terminal (y eso cuenta como Mirarla).

## 6. Interfaz

- **I1.** Swarm tiene una sola ventana. Si se abre de nuevo, se enfoca la existente.
- **I2.** La ventana tiene un **sidebar** a la izquierda con los Proyectos (y Sueltas) y sus Terminales, y a la derecha **una sola Terminal visible** a la vez.
- **I3.** Cada Terminal en el sidebar muestra el indicador de su estado y su Nombre en la línea principal, con el Título abajo y más chico. Si no tiene Nombre, muestra solo el Título. Si es Terminada, muestra también su código de salida.
- **I4.** Los Proyectos se pueden colapsar. Un Proyecto colapsado muestra un contador de las Terminales que Piden atención o están Terminadas sin mirar.
- **I5.** Las Terminales tienen **posición estable**: el orden es el de creación y el usuario lo puede reordenar a mano. El orden nunca cambia solo por el estado.
- **I6.** Una Terminal Terminada sigue mostrando su última salida hasta que se cierra.

## 7. Atajos

- **A1.** Un atajo global de la app (propuesto: `Ctrl+Shift+A`) salta a la próxima Terminal que **Pide atención**. Si no hay ninguna, salta a la próxima **Terminada sin mirar**. Si tampoco hay, no hace nada.

## 8. Persistencia y ciclo de vida

- **V1.** Los procesos de las Terminales viven en un **daemon**, no en la ventana. Ver ADR 0001.
- **V2.** Al abrir la app, si el daemon no está corriendo, la app lo arranca.
- **V3.** Cerrar la ventana no afecta al daemon ni a las Terminales.
- **V4.** Al reabrir la ventana, cada Terminal viva se reconecta. El daemon repite un buffer acotado con la salida cruda reciente y se fuerza un redimensionado para que los programas a pantalla completa se redibujen.
- **V5.** Hay una acción **"Salir y cerrar todas las Terminales"**. Pide confirmación, mata todos los procesos y apaga el daemon. La Estructura se conserva: al volver a abrir swarm, las Terminales se reabren con un shell nuevo, igual que después de un reinicio (V6).
- **V6.** Después de un reinicio de la máquina (o si el daemon muere), se restaura la **Estructura**: Proyectos y Terminales con su directorio, Nombre y orden. Las Terminales se reabren con un shell nuevo y los procesos anteriores no se recuperan.

## 9. Plataforma

- Linux con GNOME (Wayland).
- Rust con gtk4-rs y vte4-rs. Ver ADR 0003.
