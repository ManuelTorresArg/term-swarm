# Swarm

Herramienta de escritorio para tener muchas terminales abiertas, agrupadas por proyecto, donde correr agentes de desarrollo sin perder de vista en qué está cada una.

## Organización

**Proyecto**:
Un nombre asociado a un directorio raíz; agrupa Terminales que trabajan sobre ese directorio.
_Avoid_: Workspace, grupo, repo

**Terminal**:
Una terminal común y corriente, con su proceso, que swarm mantiene viva y vigila. Es la única unidad de trabajo: a swarm no le importa qué programa corre adentro.
_Avoid_: Sesión, agente, tab, pane

**Disposición**:
Cómo se acomodan en la ventana las Terminales de un Proyecto (o de Sueltas): una sola, o todas a la vez en mosaico, columnas, filas o principal y pila. Es una preferencia de la ventana, no parte de la Estructura.
_Avoid_: Layout, split, tiling

**Sueltas**:
La agrupación implícita de las Terminales que no pertenecen a ningún Proyecto.
_Avoid_: Scratch, sin proyecto, default

## Estado de una Terminal

**Activa**:
La Terminal produjo salida recientemente.

**Quieta**:
La Terminal está viva pero lleva un rato sin producir salida.
_Avoid_: Idle, inactiva, dormida

**Pide atención**:
El programa de la Terminal pidió explícitamente que lo mires. Persiste hasta que mirás esa Terminal.
_Avoid_: Esperando input, bloqueada

**Terminada**:
El proceso de la Terminal salió; se conoce su código de salida. Sigue visible hasta que el usuario la cierra.
_Avoid_: Muerta, cerrada

**Título**:
El texto que el programa de la Terminal declara como su descripción actual; funciona como resumen de qué está haciendo.
_Avoid_: Resumen, label

**Nombre**:
Etiqueta opcional que el usuario le pone a una Terminal para expresar su intención. Es distinta del Título: el Nombre lo pone el usuario y el Título lo declara el programa.
_Avoid_: Alias, label

**Reanudación**:
Comando opcional de una Terminal que swarm escribe en su shell cuando la reabre después de un reinicio, para retomar lo que corría ahí (por ejemplo, la sesión de un agente). Lo guarda el usuario o el propio programa; swarm no sabe qué significa.
_Avoid_: Restore, script de inicio, comando de arranque

**Mirar**:
Tener una Terminal visible con la ventana de swarm enfocada. Si hay varias en pantalla, solo se Mira la que tiene el foco del teclado. Es lo único que despeja Pide atención.
_Avoid_: Ver, seleccionar, abrir

## Ciclo de vida

**Estructura**:
El conjunto de Proyectos y Terminales (con su directorio, Nombre y Reanudación), sin los procesos. Es lo único que sobrevive a un reinicio de la máquina.
_Avoid_: Layout, workspace
