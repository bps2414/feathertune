# feathertune

*English: [README.md](README.md)*

> **Estável para uso diário (0.5.x).** Um projeto pessoal, usado todo dia e atualizado pelos releases do GitHub; o app instala as versões novas sozinho. Não há suporte formal.

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
- Tradução da letra: uma tradução linha a linha embaixo de cada linha (no idioma do Windows por padrão), a da própria fonte quando ela tem, senão uma tradução automática feita para letras; também na letra flutuante e no mini player.
- Letra flutuante: a linha atual sobre a área de trabalho, deixando o clique passar, palavra por palavra quando a letra tem tempo por palavra.
- Compartilhar letra como cartão, por um botão na barra de título: escolha quaisquer linhas (pule as que não quiser), escolha um visual, copie ou salve.
- O mini player também mostra a letra: a linha atual, palavra por palavra, e a próxima.
- Anterior / play-pause / próxima na miniatura da barra de tarefas e o progresso da faixa no botão da barra.
- Um menu da bandeja no visual do app, com a faixa e os controles.
- Estatísticas de escuta, guardadas só neste PC: tempo ouvido, artistas, faixas e álbuns mais ouvidos, quando você ouve, sequências; exportação CSV.
- Modo ambiente: a capa desfocada e brilhante atrás do site.
- Botões de voltar e avançar na barra de título.
- Modo jogo: um botão na barra de título que desliga na hora os recursos medidos como os mais caros (letra sincronizada na página, modo ambiente, janelas de letra flutuante e de compartilhar), e os traz de volta num segundo clique.
- Exportar diagnóstico (Configurações > Advanced): um arquivo de texto para mandar quando algo der errado.
- Modo música: escolhe a versão só áudio quando a faixa também tem vídeo, para o vídeo não ser decodificado.
- Volume preciso: a roda do mouse sobre o player muda o volume em 2% (10% com Shift), com teclas globais de volume opcionais.
- Pula faixas que começam já marcadas com "não gostei" (nunca durante um anúncio).
- Um filtro de privacidade para analytics de terceiros (nunca anúncios).
- Áudio só neste PC, no som da própria página, com todo efeito em bypass durante anúncio: equalizador de 10 bandas, compressor, crossfade entre faixas (com eco de saída opcional), pular silêncio no começo e no fim da faixa e um visualizer embaixo da capa.
- Análise de faixa: tempo, tom e contorno de volume de cada faixa, medidos na primeira vez que ela toca; só esses números ficam guardados, nunca áudio.
- Mixes: por playlist e por par de faixas, uma transição automática ou editada à mão (pontos de saída e entrada, duração, curva, eco, filtro, troca de graves, tempo) na seção Mix da aba Experimental da janela de configurações. Quando a conta deixa, o começo da próxima faixa toca numa segunda voz oculta e as duas se sobrepõem; senão a faixa some e a próxima entra.
- Controle remoto pelo celular (prévia altamente experimental): um app Android, pareado uma vez por um QR code confirmado no PC, mostra o que toca, a capa e a fila, e controla play/pause, anterior/próxima, busca, volume ao vivo, tocar ao toque, edição da fila (tocar a seguir, remover) e tocar uma playlist da sua biblioteca. Só na rede local, criptografado de ponta a ponta (Noise Protocol), nunca durante um anúncio, exceto pausar. O APK vai anexado a cada release; o app está em [mobile/](mobile/README.pt-BR.md), o protocolo em [docs/REMOTE.pt-BR.md](docs/REMOTE.pt-BR.md).
- Todo recurso vem ligado e é um plugin que você desliga na janela de configurações, menos os experimentais (crossfade, mixes, pular silêncio, análise de faixa, controle remoto), que vêm desligados na aba Experimental. Depois de uma atualização as configurações abrem no que mudou. As animações seguem esses interruptores, não os efeitos de animação do Windows, e pausam com a janela escondida, minimizada ou em segundo plano.

O que ainda não foi conferido está em [docs/DESIGN.pt-BR.md](docs/DESIGN.pt-BR.md#ainda-não-conferido).

## Instalar

Baixe `feathertune_<versão>_x64-setup.exe` nos [releases](https://github.com/bps2414/feathertune/releases) e rode: por usuário, sem admin, Windows 10/11. Ao desinstalar, ele pergunta se também apaga seus dados (login, `config.toml`); ficam guardados se a caixa não for marcada.

O APK Android do controle remoto (`feathertune-remote-<versão>.apk`) vai anexado aos releases que o alteram. Ligue o Controle pelo celular na aba Experimental das configurações, no PC, e escaneie o QR code com o app do celular.

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
discord = true               # um release já traz um id; senão precisa do client_id abaixo
lastfm = true                # também precisa conectar a conta
mini_player = true
animated_title_bar = true     # cores derivando devagar; pausa escondida, minimizada ou em segundo plano
album_color = true            # barra de título, mini player e site com as cores da capa (uma vez por faixa)
synced_lyrics = true          # letras sincronizadas na aba Letra, rolando no ritmo; algumas requisições por faixa nova, com cache
lyrics_translation = true     # uma tradução embaixo de cada linha da letra; uma requisição por faixa nova, com cache
lyric_effects = true          # pontinhos que se enchem na introdução e nas pausas longas da letra
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
equalizer = true              # plugins de áudio: um grafo Web Audio na página, em bypass durante anúncio
compressor = true
crossfade = false             # experimental (desligado por padrão); também os mixes e a voz de transição
track_analysis = false        # experimental; tempo, tom e contorno de volume em audio-facts/ no perfil
skip_silence = false          # experimental
lyric_colors = false          # experimental; uma palavra cantada que é nome de cor (red, azul...) brilha nela, estrofes nas cores do álbum
lyric_pulse = false           # experimental; o brilho da linha cantada pulsa na batida (precisa de track_analysis)
remote = false                # experimental; controle remoto: abre uma porta TCP só para celulares pareados (veja [remote])
visualizer = true

[shortcuts]                  # globais; "" desliga um
play_pause = "Ctrl+Alt+Space"
next = "Ctrl+Alt+Right"
previous = "Ctrl+Alt+Left"
mini_player = ""             # abre/fecha o mini player
volume_up = ""               # volume +2% (5% sem precise_volume)
volume_down = ""

[discord]
client_id = ""               # um release já traz o seu; ponha o seu para usá-lo (ou num build local)

[lastfm]
api_key = ""                 # opcional: os builds de release trazem uma chave própria; estes valores têm prioridade
api_secret = ""

[lyrics]
providers = ["betterlyrics", "unison", "lrcmux", "lrclib"]  # fontes de letras; as palavra por palavra vencem, depois esta ordem
estimate_words = true        # estima o tempo das palavras quando a fonte só marca linhas inteiras
offset_ms = 0                # move a letra no tempo; positivo: mais tarde
translate_to = "auto"        # idioma da tradução: auto (idioma do Windows), off, pt, en, es, fr, de, it, ja, ko
romanize = false             # a romanização da fonte (ex.: japonês, coreano) embaixo da linha
size = "medium"              # texto da aba Letra: small, medium, large
align = "left"               # left, center ou right
duets = false                # um dueto por voz: a primeira à esquerda, a segunda à direita
glow = "normal"              # off (mais leve), normal, strong

[audio]                      # a aba Áudio; os mixes ficam em mixes/<playlist>.json no perfil (aba Experimental)
eq_preset = "flat"           # flat (fora do caminho do som), bass, vocal, treble, loudness, custom
eq_bands = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]  # custom: dB de -12 a 12, 31 Hz ... 16 kHz
compressor = "none"          # none (fora do caminho do som), soft, night, strong
crossfade_ms = 6000          # 0 a 12000; 0: desligado
curve = "equal_power"        # linear, equal_power, s_curve, exponential
fade_in_ms = 1500            # 0 a 5000, entrando na próxima faixa
echo_out = false             # um eco no fim da faixa que some
visualizer = "bars"          # bars, wave

[remote]                     # o controle remoto pelo celular (aba Experimental)
port = 47652                 # TCP, 1024 a 65535; só origens da rede local
name = "feathertune"         # como os celulares pareados chamam este PC (nunca o nome do Windows)

[ui]
language = "auto"            # auto (idioma do Windows: português vira pt-BR, o resto inglês), en, pt-BR; vale depois de reiniciar

[browser]
disable_features = []        # features extras do Chromium a desligar (experimentos, veja docs/DESIGN.pt-BR.md)
```

As chaves antigas `[injections] controls`, `[privacy] tracker_filter` e `[notifications] track_change` ainda são lidas; a janela de configurações as troca pela entrada em `[plugins]`. "Iniciar com o Windows", "Conectar Last.fm" e o mini player também ficam no menu da bandeja. Um build local sem chave do Last.fm precisa de `api_key` e `api_secret` aqui. O login do Last.fm acontece no navegador; a session key fica no Windows Credential Manager, nunca em arquivo.

## Contribuindo

Pull requests são bem-vindos; abra uma issue antes para qualquer coisa grande. O guia completo (regras, disciplina de build, o que não é aceito, como funciona a revisão) é o [CONTRIBUTING.pt-BR.md](CONTRIBUTING.pt-BR.md). Problemas de segurança: [SECURITY.md](SECURITY.md).

## Aviso

Este é um projeto independente e não oficial. Não tem afiliação, endosso nem patrocínio de nenhum serviço de streaming de música nem de seus donos. Todas as marcas pertencem aos respectivos donos. Você é responsável por usá-lo de acordo com os termos do serviço em que faz login.

Licenciado sob a [Licença MIT](LICENSE).
