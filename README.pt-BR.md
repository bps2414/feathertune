# feathertune

*English: [README.md](README.md)*

> **Em desenvolvimento, experimental.** É um projeto pessoal que ainda estou testando muito. Coisas quebram, mudam ou são reescritas sem aviso. Ainda não há releases nem suporte; o CI gera um instalador; só a instalação e a desinstalação silenciosas foram testadas. Use por sua conta e risco.

Uma casca desktop leve para Windows de um player de música na web, escrita em Rust com Tauri 2. Em vez de embutir um Chromium (como os apps Electron), usa o runtime WebView2 que já vem no Windows e cuida das partes de desktop de forma nativa: ícone na bandeja, teclas de mídia e o flyout de mídia do Windows, e redução de memória enquanto escondido.

Medido até agora (árvore de processos, tocando): ~410 MB de memória privada e ~3% de um núcleo com a janela aberta, ~170 MB de RAM na bandeja, cerca de 2 s para abrir. Detalhes em [docs/PLAN.pt-BR.md](docs/PLAN.pt-BR.md).

## O que faz e o que não faz

- Carrega o **site oficial** do serviço, sem modificações. Você faz login no próprio site; o app nunca vê nem guarda suas credenciais.
- Lê a faixa atual da página só para mostrá-la no flyout de mídia e na bandeja.
- **Não** bloqueia nem altera anúncios, não baixa nem extrai áudio, não usa APIs privadas e não contorna nenhuma proteção. Alguns recursos podem exigir assinatura do serviço, exatamente como no navegador.

## Situação

| Fase | Estado |
|---|---|
| 1. Casca: janela, bandeja, redução de memória | concluída |
| 2. Ponte de estado do player, teclas de mídia, flyout | concluída |
| 3. Integrações (instância única, autostart, atalhos, Discord, Last.fm) | bandeja e atalhos conferidos; Discord e Last.fm ainda a confirmar num perfil real |
| 4. Dieta e instalador | medida (~400 MB privados, ~3,3% de CPU, ~190 MB na bandeja); instalação/desinstalação silenciosa conferida; comparação com o Electron e o caminho "excluir dados" da desinstalação em aberto |

## Compilar

Windows 10/11 com o runtime WebView2 e uma toolchain Rust (stable, MSVC).

```powershell
cd app
cargo build --release
.\target\release\feathertune.exe
```

`cargo check` é o ciclo rápido enquanto edita; `cargo build --profile dist` é o build lento e totalmente otimizado, para releases.

Instalador (NSIS, por usuário, sem admin): `cd app; npx @tauri-apps/cli@2.12.3 build -- --profile dist`, ou baixe da execução do CI. Ao desinstalar, ele pergunta se também apaga seus dados (login, `config.toml`); ficam guardados se a caixa não for marcada.

## Configuração

`%LOCALAPPDATA%\feathertune\config.toml` (criado na primeira execução só com os interruptores de injeção, fora do repositório). Acrescente as seções de que precisar; tudo é opcional:

```toml
[shortcuts]                  # globais; "" desliga um
play_pause = "Ctrl+Alt+Space"
next = "Ctrl+Alt+Right"
previous = "Ctrl+Alt+Left"

[notifications]
track_change = true          # aviso na troca de faixa, só com a janela na bandeja

[discord]
client_id = ""               # o id do seu próprio aplicativo do Discord; sem ele o Rich Presence fica desligado

[lastfm]
api_key = ""                 # sua própria conta de API do Last.fm; desligado se um dos dois estiver vazio
api_secret = ""

[browser]
disable_features = []        # features extras do Chromium a desligar (experimentos, veja docs/PLAN.pt-BR.md)

[privacy]
tracker_filter = true        # bloqueia requisições de analytics de terceiros (padrão); nunca anúncios (docs/PLAN.pt-BR.md)
```

Os itens "Start with Windows" e "Connect Last.fm" ficam no menu da bandeja. O login do Last.fm acontece no navegador; a session key fica no Windows Credential Manager, nunca em arquivo.

## Aviso

Este é um projeto independente e não oficial. Não tem afiliação, endosso nem patrocínio de nenhum serviço de streaming de música nem de seus donos. Todas as marcas pertencem aos respectivos donos. Você é responsável por usá-lo de acordo com os termos do serviço em que faz login.

Ainda não há licença escolhida, então por enquanto todos os direitos são reservados.
