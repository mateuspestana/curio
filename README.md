# Curió

TUI para monitorar mudanças em URLs — avisa na interface e no sistema, guarda histórico de diffs e abre o site no browser.

**Autor:** Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

## Requisitos

- Rust 1.90+ (toolchain estável recente)
- Modo `js` (opcional): Chrome, Chromium, Brave ou Edge instalado (ou `CURIO_CHROME=/caminho`)
- macOS / Linux (notificações via `notify-rust`)

## Build

```bash
cargo build --release
./target/release/curio
```

Ou em desenvolvimento:

```bash
cargo run
```

Binários pré-compilados na [página de releases](https://github.com/mateuspestana/curio/releases).

Interface e mensagens em **português**. Dados em `~/.curio/curio.db`.

## Uso

| Tecla | Ação |
|-------|------|
| `a` | Adicionar monitor |
| `e` | Editar selecionado |
| `d` | Excluir selecionado |
| `p` | Pausar / retomar |
| `r` | Checar agora |
| `o` | Abrir URL no browser |
| `h` | Histórico / diffs |
| `l` | Painel de alertas |
| `m` | Marcar como lido |
| `t` | Tema claro/escuro |
| `c` | Densidade normal/compacta |
| `/` | Buscar por rótulo, URL ou tag |
| `g` | Filtrar por tag (repita para a próxima) |
| `x` | Exportar monitores (TOML); no histórico, CSV e Markdown |
| `F5` | Prévia do texto (no formulário) |
| `c` / `s` | No histórico: contexto / lado a lado |
| `?` | Ajuda |
| `q` | Sair |

Intervalos: `10s`, `50m`, `2h`, `2d`, `1w` (mínimo 5s).

Detecção usa **texto visível normalizado** (um item por linha: o histórico mostra só os itens que mudaram), com seletor CSS/XPath opcional e regex de linhas a ignorar. Em falha de rede/HTTP, Curió tenta de novo até 2 vezes com intervalo de **5 minutos**; se as 3 tentativas falharem, notifica downtime.

## Novidades da v0.4.0

- **Modos**: `html` (padrão), `json` (seletor = caminho como `items[*].price`) e `js` (renderiza JavaScript com Chrome; não aceita cabeçalhos extras).
- **Alertar só se…**: `contém:`, `não contém:`, `regex:`, `número < 100`. Dispara na transição falso→verdadeiro; o histórico guarda todas as mudanças. Números no padrão brasileiro (`1.234,56`).
- **Cabeçalhos HTTP** por monitor, com `${VAR}` lido do ambiente (o banco guarda só o modelo).
- **Backoff exponencial** (60s, 120s… até 1h), respeita `Retry-After`; avisa uma vez ao cair e uma ao voltar.
- **Retenção**: `curio config set max-changes|max-days|max-snapshots N`, `curio prune`.
- **CLI**: `curio export`, `import`, `history -f csv|md`, `config`, `prune`. `CURIO_DB` troca o banco.
- Variáveis: `CURIO_NO_NOTIFY=1` desliga notificações do sistema.

## Licença

[GNU GPL v3](LICENSE) (GPL-3.0-only).

Copyright (c) 2026 Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda
