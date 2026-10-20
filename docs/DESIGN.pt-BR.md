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
├─ Discord, Last.fm, avisos de faixa, letras LRCLIB       esquema próprio, sem acesso ao site
├─ cores do álbum (decodificação WIC da capa)
└─ filtro de rastreadores (só listas de privacidade)
```

- **Janela:** a `main` não tem moldura do sistema; o Rust pinta uma faixa de 32 DIP (nome do app, faixa atual, botões de mini player e configurações, botões de legenda) e responde `WM_NCHITTEST` de forma nativa, então snap layouts, arrastar e redimensionar são do próprio Windows. O site é uma webview filha abaixo da faixa.
- **Memória:** escondido (bandeja ou mini player) põe `MemoryUsageTargetLevel=Low` no WebView2; visível põe `Normal`. A música continua.
- **Ponte:** a injeção `core` observa `navigator.mediaSession` e o `<video>` e manda ao Rust título, artista, capa, tempo e estado, só em troca de faixa, play/pause e seek, mais uma correção a cada 15 s (mandar 1×/s custava ~3% de CPU); o Rust extrapola a posição. O Rust trata o payload como não confiável: valida e nunca age por ele. Comandos Rust → página clicam nos botões do player, então um redesenho do site quebra uns três seletores CSS, não um protocolo.
- **Dados para a página:** os módulos recebem só dados, como `window.__feathertune.<módulo>.set(JSON.parse("…"))`, nunca código montado de strings. O último dado de cada módulo é reenviado depois de um recarregamento.
- **Flags do Chromium:** toda feature vai no único `--disable-features` (o Chromium só respeita o último).

### Segurança

- A capability da webview do site (`capabilities/player-bridge.json`) dá só `player_state`, só à webview `site` e só para a origem do site. `frontendDist` não é usado: com ele, o Tauri trata o site remoto como origem local.
- A webview de configurações (`capabilities/settings-window.json`) é `local: true` e dá só os três comandos `settings_*`. O Tauri casa capabilities por label **e** origem, então nenhuma página alcança os comandos da outra; nenhuma tem permissões `core:*`. Os comandos também conferem o label de quem chama.
- Esquemas próprios são registrados em toda webview (limitação do Tauri), então o handler responde 403 a tudo que não for a webview de configurações. A página de configurações tem CSP estrita e monta o DOM só com `textContent`.
- A navegação fica presa aos domínios do site e de login; o resto abre no navegador padrão.

## Plugins

Todo recurso opcional é um plugin em `app/src/plugins.rs`, com um interruptor em `[plugins]` no `config.toml` e na janela de configurações. Nada é carregado de DLL nem baixado.

| Plugin | Padrão | Custo |
|---|---|---|
| `controls` (aperta os botões do player) | ligado | nenhum mensurável |
| `tracker_filter` | ligado | economiza ~16 MB de WS e um pouco de CPU |
| `track_notifications`, `global_shortcuts` | ligado | nenhum mensurável |
| `discord`, `lastfm` | ligado (precisam de um id / uma conta) | não medido: uma thread parada cada |
| `mini_player` | ligado | nada enquanto fechado |
| `animated_title_bar` | ligado | ainda não medido |
| `album_color` | ligado | +1 MB privado, dentro do ruído |
| `synced_lyrics` (LRCLIB) | ligado | ~+5 MB, dentro da variação (uma rodada) |
| `ambient_mode` | **desligado** | ~+25 MB privados, +15 MB de WS (uma rodada) |

**Filtro de rastreadores:** `WebResourceRequested` do WebView2 e o crate `adblock` sem regras cosméticas, então ele só consegue bloquear requisições. Só vê scripts, imagens, XHR/fetch, pings e "outros" (nunca documentos, frames, mídia, fontes ou folhas de estilo), e pula os domínios do player, todo domínio do Google e os domínios de entrega de anúncios do Google antes de consultar a lista, então nem uma lista de anúncios colocada por engano muda algo ali. A lista embutida (`app/filters/privacy.txt`) tem só analytics de terceiros; neste site ela bloqueia um host.

**Last.fm:** autenticação desktop (`auth.getToken` → navegador → `auth.getSession`, consultado em intervalos); a session key fica no Gerenciador de Credenciais do Windows. Os builds de release da CI embutem a API key do mantenedor a partir dos secrets `FEATHERTUNE_LASTFM_API_KEY` / `_SECRET` do repositório (`option_env!`); chaves no `config.toml` têm prioridade. O ponto do scrobble conta só o tempo tocado; scrobbles que falham esperam em `lastfm-queue.json` (nova tentativa a cada 5 min, descartados depois de 14 dias).

**Letras sincronizadas:** um `GET https://lrclib.net/api/get` por faixa (título, artista, álbum, duração), em cache em `lyrics-cache/` no perfil; mostradas na aba Letra do site só quando o LRCLIB tem linhas sincronizadas. Entradas em Enhanced LRC (um `<mm:ss.xx>` antes de cada palavra) acendem palavra por palavra, no próprio `timeupdate` da página (umas 4 vezes por segundo, sem timer extra); a maioria das entradas é sincronizada só por linha. Outras fontes com tempo por palavra (Musixmatch, NetEase, Apple Music, ou proxies delas) são APIs privadas e ficam de fora.

## Regras de desempenho da UI

Desenhar só quando muda. Toda animação tem um interruptor de plugin e roda num único timer, só enquanto há algo na tela para mover: para quando a janela está escondida, minimizada ou em segundo plano, e quando os efeitos de animação do Windows ou o `prefers-reduced-motion` estão desligados, e retoma de onde parou. Invalidar só a região que muda. Guardar recursos de GPU (pincéis, layouts, bitmaps) entre quadros. Medir um recurso antes de ligá-lo por padrão: ele só fica ligado se custar menos que a variação da medição.

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
- Uma base no mesmo dia para saber se a casca nativa custa memória e CPU, e o plugin `animated_title_bar` medido ligado e desligado.

## Ideias

| Ideia | Tipo |
|---|---|
| Mais visuais (fontes, modo compacto, capa grande) | CSS |
| Menu da bandeja no tema do app (hoje é o menu nativo do Windows) | Rust (Win32) |
| Botões de mídia na miniatura da barra de tarefas | Rust (Win32) |
| Equalizador e compressor (Web Audio no `<video>`): antes decidir se quebra o "nunca mexer no que o player toca" | JS |

## Referências

- WebView2: [modelo de processos](https://learn.microsoft.com/microsoft-edge/webview2/concepts/process-model), [desempenho](https://learn.microsoft.com/microsoft-edge/webview2/concepts/performance), [MemoryUsageTargetLevel](https://learn.microsoft.com/dotnet/api/microsoft.web.webview2.core.corewebview2.memoryusagetargetlevel)
- Tauri 2: [WebviewWindowBuilder](https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindowBuilder.html)
- Login embutido no WebView2: [WebView2Feedback #1584](https://github.com/MicrosoftEdge/WebView2Feedback/issues/1584)
- Last.fm [autenticação desktop](https://www.last.fm/api/desktopauth); [API do LRCLIB](https://lrclib.net/docs)
