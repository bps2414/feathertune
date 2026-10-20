# feathertune

*English: [README.md](README.md)*

> **Em desenvolvimento, experimental.** É um projeto pessoal que ainda estou testando muito. Coisas quebram, mudam ou são reescritas sem aviso. Ainda não há releases nem suporte; o CI gera um instalador; só a instalação e a desinstalação silenciosas foram testadas. Use por sua conta e risco.

Uma casca desktop leve para Windows de um player de música na web, escrita em Rust com Tauri 2. Em vez de embutir um Chromium (como os apps Electron), usa o runtime WebView2 que já vem no Windows e cuida das partes de desktop de forma nativa: barra de título própria, ícone na bandeja, teclas de mídia e o flyout de mídia do Windows, um mini player sempre no topo e redução de memória enquanto escondido.

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
| 5. Casca nativa: barra de título própria, janela de configurações e plugins, mini player | feita e medida (~430 MB privados, ~4,5% de CPU aberta, acima da Fase 4 até uma base no mesmo dia); snap/150%/segundo monitor e o login do Last.fm num build do CI a confirmar pelo mantenedor |
| 6. Visual: cor do álbum, letras sincronizadas, modo ambiente | feita e medida (~433 MB privados, ~4,4% de CPU aberta com cor do álbum e letras ligadas; modo ambiente desligado por padrão, cerca de +25 MB); fluxo animado, modo claro e escala de 150% a confirmar pelo mantenedor |

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

`%LOCALAPPDATA%\feathertune\config.toml` (criado na primeira execução com a lista de plugins, fora do repositório). A janela de configurações (menu da barra de título ou da bandeja > Settings) edita tudo abaixo e mantém seus comentários; editar à mão também funciona. Tudo é opcional:

```toml
[plugins]                    # todo recurso opcional; a janela de configurações mostra descrição e custo
controls = true              # módulo da página que aperta os botões do player
tracker_filter = true        # bloqueia requisições de analytics de terceiros; nunca anúncios (docs/PLAN.pt-BR.md)
track_notifications = true   # aviso na troca de faixa, só com a janela escondida
global_shortcuts = true
discord = true               # também precisa do client_id abaixo
lastfm = true                # também precisa conectar a conta
mini_player = true
animated_title_bar = true     # cores derivando devagar; pausa escondida, minimizada ou em segundo plano
album_color = true            # barra de título, mini player e site com as cores da capa (uma vez por faixa)
synced_lyrics = true          # letras sincronizadas do LRCLIB na aba Letra; uma requisição por faixa, com cache
ambient_mode = false          # capa desfocada e brilhante atrás do site; desligado por padrão

[shortcuts]                  # globais; "" desliga um
play_pause = "Ctrl+Alt+Space"
next = "Ctrl+Alt+Right"
previous = "Ctrl+Alt+Left"
mini_player = ""             # abre/fecha o mini player

[discord]
client_id = ""               # o id do seu próprio aplicativo do Discord

[lastfm]
api_key = ""                 # opcional: os builds do CI trazem uma chave própria; estes valores têm prioridade
api_secret = ""

[browser]
disable_features = []        # features extras do Chromium a desligar (experimentos, veja docs/PLAN.pt-BR.md)
```

As chaves antigas `[injections] controls`, `[privacy] tracker_filter` e `[notifications] track_change` ainda são lidas; a janela de configurações as troca pela entrada em `[plugins]`. "Start with Windows", "Connect Last.fm" e o mini player também ficam no menu da bandeja. Um build local sem chave do Last.fm precisa de `api_key` e `api_secret` aqui. O login do Last.fm acontece no navegador; a session key fica no Windows Credential Manager, nunca em arquivo.

## Aviso

Este é um projeto independente e não oficial. Não tem afiliação, endosso nem patrocínio de nenhum serviço de streaming de música nem de seus donos. Todas as marcas pertencem aos respectivos donos. Você é responsável por usá-lo de acordo com os termos do serviço em que faz login.

Ainda não há licença escolhida, então por enquanto todos os direitos são reservados.
