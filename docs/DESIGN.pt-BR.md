# feathertune: design

*English: [DESIGN.md](DESIGN.md). Mantenha os dois arquivos em sincronia.*

Como o app é feito, por quê, quanto custa e o que ainda não foi conferido. O histórico de como chegou aqui está no log do Git.

## Abordagem

O app abre o **site oficial** do serviço de música numa janela WebView2 enxuta e faz tudo do lado do Windows em Rust (Tauri 2). Não usa API privada nem a API oficial de dados, então não quebra quando o serviço muda o protocolo, e login e assinatura funcionam como num navegador.

| Abordagem | Peso | Quebra? | Problema |
|---|---|---|---|
| Cliente nativo em APIs privadas | ~100 MB | sim, sempre | Corrida constante contra mudanças de protocolo e antiabuso, e contra os termos de uso |
| API oficial de dados + player embutido | ~Chromium | não | Cotas mínimas; as políticas proíbem tocar em segundo plano ou só áudio |
| **Site oficial num WebView2 enxuto** | ~450 MB | não (é o próprio site) | Continua sendo Chromium por dentro |

Um motor próprio também foi descartado: a página é a maior parte do peso (WebView2 vazio ~121 MB privados, o site tocando ~450 MB), CEF e Electron levam o próprio Chromium, o Servo ainda não tem Media Source Extensions completo, não existe Gecko embutível, e um cliente nativo precisa da API privada.

**Regras de base** (permanentes):
- O site não é alterado no que toca. Nada de APIs privadas, extração de stream, download ou mexer em DRM.
- Anúncios nunca são bloqueados, escondidos ou alterados.
- Nada de trocar user agent nem flags de automação (também disparam bloqueio de login).
- O app nunca digita nem guarda credenciais; o login é no site.

## Arquitetura

```
feathertune.exe (Rust, Tauri 2)                      WebView2 (o Chromium do sistema)
├─ barra de título e mini player nativos (Direct2D)  ├─ webview do site: o site oficial
├─ bandeja, fechar = esconder (memória Low)          │   └─ módulos injetados: leem o estado do player,
├─ estado do player  ◄──── ponte de mão única ───────┘      apertam os botões dele, CSS de visual
├─ teclas de mídia / flyout de mídia do Windows (SMTC)
├─ instância única, autostart, atalhos globais        └─ webview de configurações: página nossa num
├─ Discord, Last.fm, avisos de faixa, fontes de letras   esquema próprio, sem acesso ao site
├─ cores do álbum (decodificação WIC da capa)
├─ botões da barra de tarefas, menu da bandeja no tema, letra flutuante (Direct2D)
├─ estatísticas de escuta (um arquivo no perfil)
└─ filtro de rastreadores (só listas de privacidade)
```

- **Janela:** a `main` não tem moldura do sistema; o Rust pinta uma faixa de 32 DIP (nome do app, faixa atual, botões de compartilhar letra (só quando a faixa tem letra sincronizada), mini player e configurações, botões de legenda) e responde `WM_NCHITTEST` de forma nativa, então snap layouts, arrastar e redimensionar são do próprio Windows. O site é uma webview filha abaixo da faixa. Quando a página entra em tela cheia (o botão do próprio player; `ContainsFullScreenElementChanged` do WebView2) a janela entra em tela cheia junto e o site ocupa o lugar da faixa; sair (ou esconder a janela, o que faz a página sair) restaura os dois. O site é reposicionado ao mostrar, numa mudança de DPI ou de tela e nas mudanças de tela cheia, não só no `WM_SIZE`. O menu da bandeja tem "Reload page" como saída para uma página travada.
- **Memória:** escondido (bandeja ou mini player) põe `MemoryUsageTargetLevel=Low` no WebView2; visível põe `Normal`. A música continua.
- **Ponte:** a injeção `core` observa `navigator.mediaSession` e o `<video>` e manda ao Rust título, artista, capa, tempo e estado, só em troca de faixa, play/pause e seek, mais uma correção a cada 15 s (mandar 1×/s custava ~3% de CPU); o Rust extrapola a posição. O Rust trata o payload como não confiável: valida e nunca age por ele. Comandos Rust → página clicam nos botões do player, então um redesenho do site quebra uns três seletores CSS, não um protocolo.
- **Dados para a página:** os módulos recebem só dados, como `window.__feathertune.<módulo>.set(JSON.parse("…"))`, nunca código montado de strings. O último dado de cada módulo é reenviado depois de um recarregamento.
- **Flags do Chromium:** toda feature vai no único `--disable-features` (o Chromium só respeita o último).

### Segurança

- A capability da webview do site (`capabilities/player-bridge.json`) dá só `player_state`, só à webview `site` e só para a origem do site. `frontendDist` não é usado: com ele, o Tauri trata o site remoto como origem local.
- A webview de configurações (`capabilities/settings-window.json`) é `local: true` e dá só os quatro comandos `settings_*`. O quarto, `settings_stats`, entrou para a aba Stats: devolve um resumo do histórico de escuta, que é dado do próprio usuário neste PC, só para a nossa página; recebe um período e um fuso, ambos conferidos, e não muda nada. O Tauri casa capabilities por label **e** origem, então nenhuma página alcança os comandos da outra; nenhuma tem permissões `core:*`. Os comandos também conferem o label de quem chama.
- Esquemas próprios são registrados em toda webview (limitação do Tauri), então o handler responde 403 a tudo que não for a webview de configurações. A página de configurações tem CSP estrita e monta o DOM só com `textContent`.
- A navegação fica presa aos domínios do site e de login; o resto abre no navegador padrão.

## Plugins

Todo recurso opcional é um plugin em `app/src/plugins.rs`, com um interruptor em `[plugins]` no `config.toml` e na janela de configurações. Todo plugin vem ligado; o interruptor é o controle. Nada é carregado de DLL nem baixado.

| Plugin | Padrão | Custo |
|---|---|---|
| `launcher` (tela de abertura, checagem de atualização) | ligado | não medido: uma requisição pequena ao GitHub por abertura |
| `controls` (aperta os botões do player) | ligado | nenhum mensurável |
| `tracker_filter` | ligado | economiza ~16 MB de WS e um pouco de CPU |
| `track_notifications`, `global_shortcuts` | ligado | nenhum mensurável |
| `discord`, `lastfm` | ligado (precisam de um id / uma conta) | não medido: uma thread parada cada |
| `mini_player` | ligado | nada enquanto fechado |
| `animated_title_bar` | ligado | ainda não medido |
| `album_color` | ligado | +1 MB privado, dentro do ruído |
| `synced_lyrics` (LRCLIB, Better Lyrics) | ligado | ~+5 MB, dentro da variação (uma rodada); a transição e o scroll ainda não medidos |
| `ambient_mode` | ligado | ~+25 MB privados, +15 MB de WS (uma rodada) |
| `taskbar_controls` | ligado | ainda não medido: atualiza o progresso a cada 1–5 s tocando com a janela visível |
| `themed_tray_menu` | ligado | ainda não medido: nada enquanto fechado |
| `desktop_lyrics` | ligado (aberta pela bandeja) | ainda não medido: nada enquanto fechada; um redesenho por troca de linha ou palavra |
| `share_lyrics` | ligado (aberto pela barra de título, bandeja ou configurações) | ainda não medido: nada até abrir; redesenha só com entrada |
| `listening_stats` | ligado | ainda não medido: uma thread parada, uma linha gravada por faixa |

**Filtro de rastreadores:** `WebResourceRequested` do WebView2 e o crate `adblock` sem regras cosméticas, então ele só consegue bloquear requisições. Só vê scripts, imagens, XHR/fetch, pings e "outros" (nunca documentos, frames, mídia, fontes ou folhas de estilo), e pula os domínios do player, todo domínio do Google e os domínios de entrega de anúncios do Google antes de consultar a lista, então nem uma lista de anúncios colocada por engano muda algo ali. A lista embutida (`app/filters/privacy.txt`) tem só analytics de terceiros; neste site ela bloqueia um host.

**Last.fm:** autenticação desktop (`auth.getToken` → navegador → `auth.getSession`, consultado em intervalos); a session key fica no Gerenciador de Credenciais do Windows. Os builds de release da CI embutem a API key do mantenedor a partir dos secrets `FEATHERTUNE_LASTFM_API_KEY` / `_SECRET` do repositório (`option_env!`); chaves no `config.toml` têm prioridade. O ponto do scrobble conta só o tempo tocado; scrobbles que falham esperam em `lastfm-queue.json` (nova tentativa a cada 5 min, descartados depois de 14 dias).

**Letras sincronizadas:** as fontes são tentadas na ordem de `[lyrics] providers` (janela de configurações, aba Lyrics) até uma ter linhas sincronizadas; a resposta, ou um "não encontrado" para aquela lista de fontes, fica em cache em `lyrics-cache/` no perfil.
- **LRCLIB** (aberto, feito pela comunidade): `GET https://lrclib.net/api/get` (título, artista, álbum, duração). Sem linhas sincronizadas ali, `GET /api/search` com o título limpo do que títulos de vídeo acrescentam ("(Official Video)", "[Lyrics]", "feat. ...", um "Artista - " na frente, " - Remastered" atrás), depois só por artista e título (`q`) se isso não achar nada; fica o resultado de duração mais próxima (no máximo 10 s de diferença) cujo artista e título batem (normalizados, iguais ou um contendo o outro) e que não é instrumental.
- **Better Lyrics** ([API pública](https://docs.betterlyrics.org), aberta a apps de terceiros): `GET https://api.betterlyrics.org/getLyrics` (título, artista, álbum, duração), TTML sincronizado palavra por palavra; vocais de fundo ficam de fora, e uma duração diferente conta como falta. Sem chave de API (não estão emitindo) só responde faixas que já tem em cache, então um 401 é falta. As letras vêm da Apple Music por esse serviço: chamamos só a API pública documentada dele, nunca a da Apple, e por isso ele é permitido enquanto Apple Music, Musixmatch ou NetEase chamados direto (APIs privadas) ficam de fora.

Linhas sincronizadas só por inteiro ganham **tempo estimado por palavra** (`[lyrics] estimate_words`, ligado por padrão): cada palavra pesa suas sílabas (grupos de vogais, blocos Hangul; um caractere chinês ou japonês é uma palavra), +0,5 depois de vírgula e +1 na última palavra (que costuma ser segurada), e a linha é cantada no ritmo da própria música, o primeiro quartil do tempo por sílaba das suas linhas, dentro de 92% do tempo até a próxima linha, então as palavras não se esticam sobre um intervalo instrumental. Contra tempos reais por palavra (7 músicas, 2411 palavras) o início de uma palavra erra 249 ms em média (mediana 143 ms, 75% abaixo de 300 ms), contra 533 ms ao espalhar as palavras pela linha inteira pelo tamanho; conferido só em músicas em inglês. A página acende palavras estimadas com uma transição mais suave e diz isso no crédito. `[lyrics] offset_ms` move todos os tempos. Os dois valem na hora para a faixa atual (as linhas guardadas da fonte são processadas de novo).

Entradas em Enhanced LRC (um `<mm:ss.xx>` antes de cada palavra) e do Better Lyrics acendem palavra por palavra com os próprios tempos, no próprio `timeupdate` da página (umas 4 vezes por segundo, sem timer extra). A página põe sua caixa no tab renderer que mostra a página de Letra (pode haver mais de um, por exemplo com uma página de álbum aberta), conferido de novo quando qualquer `page-type` muda e nos eventos de tempo do vídeo. As linhas fazem transição cruzada (a antiga esmaece devagar, a nova entra mais rápido). Enquanto a música toca e a janela está em primeiro plano, um laço de animation frame rola a lista no ritmo: a vista estaciona na linha atual e passa para a próxima em volta da troca (de 40% da linha que termina, no máximo 1,6 s, antes dela, até 45% da nova, no máximo 1,8 s, depois, com suavização), então uma linha que termina ainda não chegou à metade do caminho e linhas curtas mantêm a vista andando; desliza suave para um novo lugar depois de um seek, um clique ou uma rolagem manual, e só escreve o scroll quando ele anda um pixel inteiro do dispositivo; nesse tempo ele também troca as linhas e palavras. Fora disso, cada linha nova rola suave até o meio. Descartados: o token do app desktop do Musixmatch, as letras sincronizadas do próprio serviço (exigem a identidade de outro cliente), Genius (só letra sem tempo, que o site já tem), Megalobiz (fora do ar, HTTP 410), a API de letras do SimpMusic (só para os clientes dele) e alinhar a letra ao áudio (leria o que o player toca).

**Letra flutuante:** uma janela layered nativa (Direct2D num bitmap de 32 bits, `UpdateLayeredWindow`) com a linha atual, acesa palavra por palavra quando a entrada tem tempo por palavra, e a próxima mais apagada, com contorno para ler sobre qualquer fundo. As linhas vêm do Rust (`lyrics::Lyrics`), então funciona com `synced_lyrics` desligado; o tempo é `position_now()` com o adiantamento de 250 ms da página. Um timer de disparo único vai para a próxima linha ou palavra, e um timer de 60 fps roda só durante a rolagem de 420 ms: a próxima linha sobe para o lugar da atual e cresce até ela enquanto a antiga some para cima e a nova próxima aparece de baixo, então duas linhas nunca ficam uma por cima da outra. Travada, os cliques passam por ela (`WS_EX_TRANSPARENT`); destravada (bandeja), mostra um painel para arrastar e um botão de travar. Some com a música pausada ou sem letra sincronizada. Aberta ou não e a posição ficam em `desktop-lyrics.json` no perfil. O tempo e o desenho das linhas (`lyric_view.rs`) são compartilhados com o mini player: o botão de letra dele o aumenta com uma faixa com a linha atual e a próxima nas cores da janela (sem contorno), mantida com a música pausada, recortada na faixa e rolada a 60 fps; se a faixa está aberta fica em `mini-player.json`. Os controles têm alvos de 30–38 DIPs (play/pausa um disco de 38 DIPs), e os de cima (letra, janela completa, fechar) ficam sobre pílulas de vidro sutis e mostram o que fazem num rótulo pequeno ao passar o mouse.

**Compartilhar letra:** uma janela nativa (Direct2D) lista as linhas sincronizadas da faixa (de `lyrics::Lyrics`, então funciona com `synced_lyrics` desligado) e mostra a prévia de um cartão. Dá para escolher quaisquer linhas, até 8, com um clique, ou um intervalo com Shift-clique; linhas puladas entre duas escolhidas viram três pontinhos no cartão. Visuais: Cover (um campo suave das cores da capa: o quadrado central reduzido a 5×5 e ampliado com filtro cúbico, pelo WIC, com os brilhos do tema e escurecido para baixo), Ink (a tinta e os brilhos do tema do álbum), Dark e Paper. O cartão tem a capa, o título e o artista em cima, umas aspas grandes, as linhas e um rodapé com uma nota desenhada e o álbum; sem nome do app. A cor dominante do álbum é o destaque (aspas, uma barra que esmaece ao lado das linhas, pontinhos de pulo, a nota), as linhas têm uma sombra suave, Ink e Dark uma vinheta, e todo visual um grão de filme fixo e sutil. Abre por um botão na barra de título que só aparece quando a faixa tem letra sincronizada (um botão nativo, então a página não ganha comando novo), pela bandeja ou pela janela de configurações. O cartão é desenhado por uma função só, em escala na prévia e num bitmap de 32 bits a 3× (1080 px de largura); ele é opaco, então vai para a área de transferência como DIB e como "PNG", e Save grava um PNG (WIC) em Downloads. Nada é enviado para lugar nenhum.

**Barra de tarefas:** `ITaskbarList3` a partir de um subclass da janela principal: anterior / play-pause / próxima na miniatura (ícones de glifo brancos desenhados uma vez com Direct2D no tamanho de ícone pequeno do DPI) e o progresso da faixa no botão da barra, amarelo quando pausado. Os botões são adicionados a cada "TaskbarButtonCreated" (janela mostrada da bandeja, Explorer reiniciado). O progresso atualiza uma vez a cada 1% da faixa (1–5 s) tocando com a janela visível.

**Menu da bandeja no tema:** um popup nativo na tinta e nos brilhos do álbum (ou nas cores do sistema) com a faixa e os controles em cima e os itens da bandeja embaixo; fecha ao perder o foco, no Esc, num clique fora ou depois de um item. Quando o Windows não lhe dá o primeiro plano, ela segura a captura do mouse e então também fecha ao perder a captura, quando o app é desativado ou (um timer de 250 ms enquanto aberto) quando a janela em primeiro plano muda, então nunca fica solta na tela. Leitores de tela não o veem como menu, ao contrário do menu do Windows: desligar o plugin traz aquele de volta.

**Estatísticas de escuta:** cada audição com pelo menos 10 s tocados (contados como o ponto do scrobble) vira uma linha JSON em `stats/plays.jsonl` no perfil; conta como play com metade da faixa ou 30 s, menos que isso é pulo. A aba Stats da janela de configurações pede a `settings_stats` um período (7 dias, 30 dias, 12 meses, tudo) e recebe totais, uma série por dia ou mês, um mapa hora × dia da semana, sequências e os artistas, faixas e álbuns mais ouvidos, calculados em Rust a partir do arquivo inteiro. Exportar grava um CSV em Downloads; "clear history" apaga o arquivo. A audição em andamento é gravada na troca de faixa e ao sair ou reiniciar pelo app; um travamento ou o desligamento do Windows a perde.

**Launcher:** numa abertura interativa (sem `--hidden`, `--mini-player` ou `--measure`) uma tela de abertura nativa pequena (Direct2D) aparece primeiro e diz o que o app está fazendo. A janela principal é criada escondida, então o site carrega enquanto isso; ela é mostrada quando a checagem de atualização termina e o site desenhou o primeiro conteúdo (uma sonda na página, no máximo 8 s depois do `load`, 20 s no total), nunca numa página em branco. A checagem é um `GET` do último release do repositório na API do GitHub. O instalador de um release mais novo é baixado, o SHA-256 dele é comparado com o que o GitHub lista para o arquivo (o instalador não é assinado, então hash ausente ou diferente interrompe a atualização) e executado em silêncio (`/S /R /UPDATE`: ele fecha o app, substitui e abre de novo). Só uma cópia instalada se atualiza (o exe está na pasta do perfil, onde o instalador o coloca); um build de `target/` nunca. Qualquer falha, inclusive sem internet, só abre a versão atual.

## Regras de desempenho da UI

Desenhar só quando muda. Toda animação tem um interruptor de plugin e roda num único timer, só enquanto há algo na tela para mover: para quando a janela está escondida, minimizada ou em segundo plano, e retoma de onde parou. Os efeitos de animação do Windows não desligam as animações; os interruptores dos plugins sim (a página recebe `--force-prefers-no-reduced-motion`, então o site e os módulos da página também animam). Invalidar só a região que muda. Guardar recursos de GPU (pincéis, layouts, bitmaps) entre quadros. Os recursos vêm ligados; meça cada um e escreva o custo na lista de plugins, para o interruptor ser uma escolha informada.

## Medições

Critério: `Win32_Process`, soma da árvore de processos; privado = `PrivatePageCount`, WS = `WorkingSetSize`; CPU em % de um núcleo em 20 s. Logado, 1280×800, uma faixa fixa tocando, 3 rodadas, mediana (mín–máx).

| Build (20/10/2026) | Cenário | CPU | Privado | WS | Início até a barra |
|---|---|---|---|---|---|
| sem a casca nativa, `dist` | janela aberta | 3,3% (3,1–3,4) | 403 MB | 733 MB | 1,9 s |
| sem a casca nativa, `dist` | na bandeja | 3,3% | 411 MB | **188 MB** | 2,0 s |
| casca nativa, `dist` | janela aberta | 4,5% (4,1–5,5) | 430 MB | 761 MB | 1,9 s |
| casca nativa, `dist` | mini player, janela principal escondida | 4,0% | 442 MB | 218 MB | 2,0 s |
| padrões atuais, `release` | janela aberta | 4,4% (3,1–4,5) | 433 MB | 765 MB | 1,9 s |

| | Cliente Electron (referência, medição anterior) | Meta |
|---|---|---|
| Janela aberta | 16–32% de CPU, 672 MB privados, 1009 MB de WS | ≤ 4% de CPU, ≤ 480 MB privados, ≤ 750 MB de WS |
| Na bandeja | igual a aberto | ≤ 260 MB de WS |
| Instalador | ~100 MB | poucos MB (feathertune: ~3 MB) |

- As linhas da casca nativa estão acima das anteriores, mas não houve base no mesmo dia, então ainda não se sabe quanto vem da casca e quanto do site ou do runtime do WebView2 mudando. CPU aberta e WS ficam um pouco acima da meta.
- Mantidos dos experimentos: sem descoberta de Cast (`MediaRouter`, `DialMediaRouteProvider`: −12 MB de WS) e o filtro de rastreadores (−16 MB de WS). Testados e descartados: sem renderer reserva, sem prerender, sem optimization hints, um módulo CSS "sem animações", a lista EasyPrivacy completa. Não testados de propósito: isolamento de sites desligado, `--disable-component-update`, `--disable-gpu` (CPU 15%), V8 lite (mais CPU), qualquer coisa ligada a anúncios.
- Para onde vai a RAM (tocando, privado): processo principal ~40 MB + ~4 MB do exe em Rust, GPU ~190 MB, renderer ~180 MB, rede/áudio/armazenamento ~33 MB. O que sobra é o Chromium mais a própria página, que nenhum wrapper corta.

### Reproduzindo

O `measure.ps1` mede qualquer exe (padrão: o build release) no perfil real, então mede logado. Feche o app antes: duas instâncias dividem o processo browser. `--measure` faz o app imprimir `player_bar_ms` e, a cada 10 s, o tempo do `<video>` e o estado do login. A saída vai para `results/` (no gitignore: tem caminhos locais e histórico de músicas).

```powershell
cd app; cargo build --release; cd ..
$a = '--measure', '<url da faixa>'
1..3 | % { ./measure.ps1 -Name open-$_ -ArgumentList $a }                          # janela aberta
1..3 | % { ./measure.ps1 -Name tray-$_ -ArgumentList $a -HideAfter 25 -Wait 50 }   # na bandeja
./measure.ps1 -Name mini -ArgumentList $a -MiniAfter 25 -Wait 50                     # mini player
./measure.ps1 -Name tray-1h -ArgumentList $a -HideAfter 25 -Minutes 60
./median.ps1 -Prefix open-
```

`-HideAfter` manda WM_CLOSE (o app vai para a bandeja); `-MiniAfter` roda `feathertune.exe --mini-player` na instância aberta; `-Exe app/target/dist/feathertune.exe` mede o build `dist` totalmente otimizado (o do instalador).

## Instalador

Bundler NSIS do Tauri, por usuário (sem admin), instalado em `%LOCALAPPDATA%\feathertune`, a mesma pasta do perfil. O bootstrapper do WebView2 só é baixado onde o runtime falta. Inglês e português do Brasil. O atalho do menu Iniciar leva o AppUserModelID `dev.feathertune.desktop` (o mesmo que o processo define, então o flyout de mídia mostra o nome do app). Desinstalar mantém o perfil, a menos que a caixa "apagar dados" esteja marcada; aí o `installer-hooks.nsh` também apaga o perfil e a credencial do Last.fm.

Enviar uma tag `v*` roda o job `release` da CI, que compila com `--profile dist` e anexa o instalador a um release do GitHub.

## Ainda não conferido

- Com mouse de verdade: snap layouts, arrastar, redimensionar e maximizar/restaurar, em 150% de escala e num segundo monitor; digitar no site depois de trocar de janela.
- Discord Rich Presence e um login e scrobble reais no Last.fm, inclusive num build da CI com a chave embutida.
- "Start with Windows" gravando a entrada Run; o aviso de faixa sendo visto; o teste de 1 h na bandeja.
- O caminho "apagar dados" do desinstalador.
- A transição de 0,7 s entre capas, o Windows em modo claro, uma falta no LRCLIB, e um recarregamento mantendo tema e letras.
- Letras: com que frequência a busca no LRCLIB e o Better Lyrics acham o que o `/api/get` não achou (e se algum resultado vem errado); o tempo estimado por palavra em músicas em português e outras línguas; a aba Letra depois de Próxima com a página do player expandida num álbum; a aba Lyrics da janela de configurações, a ordem das fontes e a busca.
- Uma base no mesmo dia para saber se a casca nativa custa memória e CPU, e o plugin `animated_title_bar` medido ligado e desligado.
- A transição e o scroll no ritmo da letra no app rodando (suavidade, rolagem manual e seeks) e o custo de CPU deles; os tempos do novo scroll que estaciona.
- No app rodando: os botões e o progresso da barra de tarefas (também depois de reiniciar o Explorer e rodando como administrador); o menu da bandeja no tema (posição em cada borda da barra, fechar com clique fora, teclado, escala de 150%); a letra flutuante (legibilidade, tempo das palavras contra o site, arrastar, segundo monitor); a aba Stats com um histórico real e a exportação CSV.
- A janela de compartilhar letra no app rodando (os visuais com capas claras e escuras, colar em apps de chat, Save, escala de 150%) e o botão dela na barra de título; a faixa de letra do mini player (o glifo de letra, o tempo, a rolagem entre linhas, redimensionar perto da borda da tela) e os botões maiores com rótulos ao passar o mouse; o menu da bandeja não ficar mais aberto depois de um clique em outro lugar.
- O botão de tela cheia do player (a janela entra em tela cheia com o site cobrindo tudo, Esc e o botão de sair da página restauram, esconder na bandeja em tela cheia), a posição do site depois de restaurar da bandeja ou mover entre monitores de DPI diferentes, e o "Reload page" da bandeja.
- O custo dos cinco plugins novos.

## Ideias

| Ideia | Tipo |
|---|---|
| Mais visuais (fontes, modo compacto, capa grande) | CSS |
| Equalizador e compressor (Web Audio no `<video>`): antes decidir se quebra o "nunca mexer no que o player toca" | JS |

## Referências

- WebView2: [modelo de processos](https://learn.microsoft.com/microsoft-edge/webview2/concepts/process-model), [desempenho](https://learn.microsoft.com/microsoft-edge/webview2/concepts/performance), [MemoryUsageTargetLevel](https://learn.microsoft.com/dotnet/api/microsoft.web.webview2.core.corewebview2.memoryusagetargetlevel)
- Tauri 2: [WebviewWindowBuilder](https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindowBuilder.html)
- Login embutido no WebView2: [WebView2Feedback #1584](https://github.com/MicrosoftEdge/WebView2Feedback/issues/1584)
- Last.fm [autenticação desktop](https://www.last.fm/api/desktopauth); [API do LRCLIB](https://lrclib.net/docs); [API do Better Lyrics](https://docs.betterlyrics.org)
