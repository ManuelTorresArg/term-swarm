# Terminales agnósticas: el estado sale solo de señales estándar de terminal

Swarm no tiene integraciones por agente (ni hooks de Claude Code ni parseo de la salida). Cada Terminal es una terminal común, y su estado sale solo de señales que cualquier programa puede emitir: actividad de la salida, salida del proceso, BEL y notificaciones OSC 9/777 (que significan "Pide atención"), y el título OSC 0/2 (que es el Título). Lo elegimos por simplicidad y para no acoplarnos a ningún agente. El precio es que un agente que no emite estas señales nunca va a aparecer como "Pide atención".

La Reanudación (R1–R5) sigue la misma idea: swarm guarda y escribe un comando opaco que el usuario o el agente le dan por `swarmctl`. La integración con un agente concreto (por ejemplo, los hooks de Claude Code) vive del lado del agente, no en swarm.
