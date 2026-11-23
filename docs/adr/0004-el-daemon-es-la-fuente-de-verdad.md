# El daemon es la fuente de verdad del estado y manda las notificaciones

El daemon no solo es dueño de los procesos: también detecta las señales (BEL, OSC), calcula el estado de cada Terminal, guarda la Estructura y manda las notificaciones de escritorio. La app es un cliente delgado que muestra lo que el daemon le dice y le avisa qué Terminal se está Mirando. Lo elegimos porque el estado tiene que seguir evolucionando con la ventana cerrada. Una Terminal que pide atención o termina mientras no hay ventana tiene que notificar (N1) y aparecer así al reabrir. Si la detección viviera en VTE, del lado de la app, eso no pasaría.

## Consequences

- El protocolo app↔daemon es un socket Unix con un mensaje JSON por línea y los bytes de terminal en base64 (crate `swarm-ipc`). Es fácil de depurar con `socat`; si el volumen de salida lo justifica, se puede pasar a un formato binario sin tocar el modelo.
- Como varios clientes pueden estar conectados a la vez, "Mirar" es por cliente: una Terminal está Mirada si algún cliente la está Mirando.
