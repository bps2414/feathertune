# feathertune

*English: [README.md](README.md)*

> **Estável para uso diário (0.1.x).** Um projeto pessoal, usado todo dia e atualizado pelos releases do GitHub; o app instala as versões novas sozinho. Não há suporte formal.

Uma casca desktop leve para Windows de um player de música na web, escrita em Rust com Tauri 2. Em vez de embutir um Chromium (como os apps Electron), usa o runtime WebView2 que já vem no Windows e cuida das partes de desktop de forma nativa: barra de título própria, ícone na bandeja, teclas de mídia e o flyout de mídia do Windows, um mini player sempre no topo e redução de memória enquanto escondido.

Medido (árvore de processos, tocando): ~430 MB de memória privada e ~4% de um núcleo com a janela aberta, ~190 MB de RAM na bandeja, cerca de 2 s para abrir, instalador de ~3 MB. Detalhes em [docs/DESIGN.pt-BR.md](docs/DESIGN.pt-BR.md).

## O que faz e o que não faz

- Carrega o **site oficial** do serviço, sem modificações. Você faz login no próprio site; o app nunca vê nem guarda suas credenciais.
- Lê a faixa atual da página só para mostrá-la no flyout de mídia e na bandeja.
- **Não** bloqueia nem altera anúncios, não baixa nem extrai áudio, não usa APIs privadas e não contorna nenhuma proteção. Alguns recursos podem exigir assinatura do serviço, exatamente como no navegador.

## Recursos

- Uma tela de abertura que procura atualizações (e instala uma versão nova) enquanto o player carrega, então a janela nunca abre numa página em branco.
- Barra de título nativa nas cores do álbum, com a faixa atual; um mini player que fica no topo.
- Ícone na bandeja, fechar para a bandeja com memória reduzida, teclas de mídia e o flyout de mídia do Windows, atalhos globais, aviso de faixa enquanto escondido.
- Discord Rich Presence e scrobbles no Last.fm (com fila offline).
- Letras sincronizadas na aba Letra do site, de quatro fontes abertas (Better Lyrics, Unison, lrcmux, LRCLIB), preferindo as sincronizadas palavra por palavra: linhas e palavras acendem conforme são cantadas (o tempo das palavras é estimado pelo ritmo da música quando nenhuma fonte o tem), uma esmaece enquanto a outra aparece, e a lista rola devagar no ritmo da música.
- Letra flutuante: a linha atual sobre a área de trabalho, deixando o clique passar, palavra por palavra quando a letra tem tempo por palavra.
- Compartilhar letra como cartão, por um botão na barra de título: escolha quaisquer linhas (pule as que não quiser), escolha um visual, copie ou salve.
- O mini player também mostra a letra: a linha atual, palavra por palavra, e a próxima.
- Anterior / play-pause / próxima na miniatura da barra de tarefas e o progresso da faixa no botão da barra.
- Um menu da bandeja no visual do app, com a faixa e os controles.
- Estatísticas de escuta, guardadas só neste PC: tempo ouvido, artistas, faixas e álbuns mais ouvidos, quando você ouve, sequências; exportação CSV.
- Modo ambiente: a capa desfocada e brilhante atrás do site.
- Botões de voltar e avançar na barra de título.
- Modo música: escolhe a versão só áudio quando a faixa também tem vídeo, para o vídeo não ser decodificado.
- Volume preciso: a roda do mouse sobre o player muda o volume em 2% (10% com Shift), com teclas globais de volume opcionais.
- Pula faixas que começam já marcadas com "não gostei" (nunca durante um anúncio).
- Um filtro de privacidade para analytics de terceiros (nunca anúncios).
- Todo recurso vem ligado e é um plugin que você desliga na janela de configurações. As animações seguem esses interruptores, não os efeitos de animação do Windows, e pausam com a janela escondida, minimizada ou em segundo plano.

O que ainda não foi conferido está em [docs/DESIGN.pt-BR.md](docs/DESIGN.pt-BR.md#ainda-não-conferido).

## Instalar

Baixe `feathertune_<versão>_x64-setup.exe` nos [releases](https://github.com/bps2414/feathertune/releases) e rode: por usuário, sem admin, Windows 10/11. Ao desinstalar, ele pergunta se também apaga seus dados (login, `config.toml`); ficam guardados se a caixa não for marcada.

O que mudou em cada versão: [CHANGELOG.pt-BR.md](CHANGELOG.pt-BR.md).

## Compilar

Windows 10/11 com o runtime WebView2 e uma toolchain Rust (stable, MSVC).

```powershell
cd app
cargo build --release
.\target\release\feathertune.exe
```

`cargo check` é o ciclo rápido enquanto edita; `cargo build --profile dist` é o build lento e totalmente otimizado, para releases. Instalador: `cd app; npx @tauri-apps/cli@2.12.3 build -- --profile dist`.

## Configuração

`%LOCALAPPDATA%\feathertune\config.toml` (criado na primeira execução com a lista de plugins, fora do repositório; um arquivo que já existe mantém os valores escritos nele). A janela de configurações (engrenagem da barra de título, ou menu da bandeja > Settings) edita tudo abaixo e mantém seus comentários; editar à mão também funciona. Tudo é opcional:

```toml
[plugins]                    # todo recurso opcional; a janela de configurações mostra descrição e custo
launcher = true              # tela de abertura: procura versão nova no GitHub, instala e espera o player
controls = true              # módulo da página que aperta os botões do player
tracker_filter = true        # bloqueia requisições de analytics de terceiros; nunca anúncios (docs/DESIGN.pt-BR.md)
track_notifications = true   # aviso na troca de faixa, só com a janela escondida
global_shortcuts = true
discord = true               # também precisa do client_id abaixo
lastfm = true                # também precisa conectar a conta
mini_player = true
animated_title_bar = true     # cores derivando devagar; pausa escondida, minimizada ou em segundo plano
album_color = true            # barra de título, mini player e site com as cores da capa (uma vez por faixa)
synced_lyrics = true          # letras sincronizadas na aba Letra, rolando no ritmo; algumas requisições por faixa nova, com cache
ambient_mode = true           # capa desfocada e brilhante atrás do site
taskbar_controls = true       # botões na miniatura da barra de tarefas, progresso no botão da barra
themed_tray_menu = true       # menu da bandeja no visual do app; false: o menu do Windows
desktop_lyrics = true         # letra flutuante sobre a área de trabalho (abra pela bandeja)
share_lyrics = true           # cartões de letra em imagem (abra pela bandeja)
listening_stats = true        # histórico em stats/plays.jsonl no perfil, mostrado na aba Stats
navigation = true             # botões de voltar e avançar na barra de título
song_mode = true              # escolhe a versão só áudio de uma faixa que também tem vídeo
precise_volume = true         # roda do mouse sobre o player: volume em 2% (Shift: 10%)
skip_disliked = true          # pula uma faixa que começa marcada com "não gostei"

[shortcuts]                  # globais; "" desliga um
play_pause = "Ctrl+Alt+Space"
next = "Ctrl+Alt+Right"
previous = "Ctrl+Alt+Left"
mini_player = ""             # abre/fecha o mini player
volume_up = ""               # volume +2% (5% sem precise_volume)
volume_down = ""

[discord]
client_id = ""               # o id do seu próprio aplicativo do Discord

[lastfm]
api_key = ""                 # opcional: os builds de release trazem uma chave própria; estes valores têm prioridade
api_secret = ""

[lyrics]
providers = ["betterlyrics", "unison", "lrcmux", "lrclib"]  # fontes de letras; as palavra por palavra vencem, depois esta ordem
estimate_words = true        # estima o tempo das palavras quando a fonte só marca linhas inteiras
offset_ms = 0                # move a letra no tempo; positivo: mais tarde

[browser]
disable_features = []        # features extras do Chromium a desligar (experimentos, veja docs/DESIGN.pt-BR.md)
```

As chaves antigas `[injections] controls`, `[privacy] tracker_filter` e `[notifications] track_change` ainda são lidas; a janela de configurações as troca pela entrada em `[plugins]`. "Start with Windows", "Connect Last.fm" e o mini player também ficam no menu da bandeja. Um build local sem chave do Last.fm precisa de `api_key` e `api_secret` aqui. O login do Last.fm acontece no navegador; a session key fica no Windows Credential Manager, nunca em arquivo.

## Aviso

Este é um projeto independente e não oficial. Não tem afiliação, endosso nem patrocínio de nenhum serviço de streaming de música nem de seus donos. Todas as marcas pertencem aos respectivos donos. Você é responsável por usá-lo de acordo com os termos do serviço em que faz login.

Ainda não há licença escolhida, então por enquanto todos os direitos são reservados.
