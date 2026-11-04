# Curió

TUI para monitorar mudanças em URLs — avisa na interface e no sistema, guarda histórico de diffs e abre o site no browser.

**Autor:** Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda

## Requisitos

- Rust 1.75+ (recomendado toolchain estável recente)
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

Binários pré-compilados em `dist/`:

| Arquivo | Plataforma |
|---------|------------|
| `curio-macos-aarch64` | macOS Apple Silicon |
| `curio-linux-x86_64` | Linux x86_64 |
| `curio-windows-x86_64.exe` | Windows x86_64 |

Interface e mensagens em **português**. Dados em `~/.curio/curio.db`.

## Uso

| Tecla | Ação |
|-------|------|
| `a` | Adicionar monitor |
| `e` | Editar selecionado |
| `d` | Excluir selecionado |
| `o` | Abrir URL no browser |
| `h` | Histórico / diffs |
| `l` | Painel de alertas |
| `m` | Marcar como visto |
| `?` | Ajuda |
| `q` | Sair |

Intervalos: `10s`, `50m`, `2h`, `2d`, `1w` (mínimo 5s).

Detecção usa **texto visível normalizado**. Em falha de rede/HTTP, Curió tenta de novo até 2 vezes com intervalo de **5 minutos**; se as 3 tentativas falharem, notifica downtime.

## Licença

[GNU GPL v3](LICENSE) (GPL-3.0-only).

Copyright (c) 2026 Matheus C. Pestana — MACAPE Pesquisas e Consultoria em Tecnologia Ltda
