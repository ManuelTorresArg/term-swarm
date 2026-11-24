# Reanudación de Claude Code

Con estos hooks, cada Terminal de swarm donde corre Claude Code guarda su
Reanudación (R3): después de un reinicio, la Terminal se reabre y retoma la
misma sesión con `claude --resume`.

Agregar en `~/.claude/settings.json` (requiere `jq`):

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "swarmctl reanudacion \"$(jq -r '\"cd \\(.cwd|@sh) && claude --resume \\(.session_id)\"')\""
          }
        ]
      }
    ],
    "SessionEnd": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "[ \"$(jq -r .reason)\" = other ] || swarmctl reanudacion --borrar"
          }
        ]
      }
    ]
  }
}
```

- **SessionStart** recibe por stdin el `session_id` y el directorio de la
  sesión, y guarda `cd '<dir>' && claude --resume <id>`. Se dispara también al
  reanudar o al hacer `/clear`, así la Reanudación siempre apunta a la sesión
  actual.
- **SessionEnd** la borra cuando salís de Claude a propósito (R4), pero no
  cuando el proceso muere por otra razón (`reason` = `other`), como al apagar
  la máquina o con "Salir y cerrar todo".
- Fuera de swarm (`$SWARM_TERMINAL` no está definida) `swarmctl reanudacion`
  no hace nada, así que los hooks son inofensivos en otras terminales.
