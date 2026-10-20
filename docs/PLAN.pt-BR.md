# feathertune: plano

*English: [PLAN.md](PLAN.md). Mantenha os dois arquivos em sincronia.*

Um app Rust (Tauri 2) que abre o **site oficial** do serviço de música num WebView2 enxuto e cuida em Rust de tudo que é do Windows: bandeja, teclas de mídia, integrações e memória. Não usa API privada nem a API oficial de dados, então não quebra quando o serviço muda o protocolo, e o login e a assinatura do usuário funcionam como no navegador.

## Por que este caminho

| Caminho | Peso | Quebra? | Problema |
|---|---|---|---|
| Cliente nativo com API privada | ~100 MB | sim, sempre | Corrida constante contra mudanças de protocolo e antiabuso, e contra os termos de uso |
| API oficial de dados + player embutido | ~Chromium | não | Cota minúscula, e a política proíbe tocar em segundo plano ou só o áudio |
| **Site oficial num WebView2 enxuto (este plano)** | ~450 MB | não (é o próprio site) | Continua sendo Chromium por dentro |

Regras que saem disso e que toda fase mantém:
- A página é o site oficial, sem mexer no que ele toca. Nada de API privada, extração de stream, download ou DRM.
- **Anúncios nunca são bloqueados, escondidos ou alterados.** Isso fica fora do escopo para sempre.
- Nada de user agent falso nem flags de automação (também disparam bloqueio de login).
- O app nunca guarda nem digita credenciais; o login acontece no site.

## Peso medido

Critério (o mesmo em tudo): Win32_Process, soma da árvore de processos; privado = PrivatePageCount, WS = WorkingSetSize; CPU em % de um núcleo numa janela de 20 s. Referência: um cliente desktop em Electron para o mesmo site, medido na mesma máquina.

Protótipo (`spike/`: wry 0.57 + WebView2 155 sem otimização, deslogado, janela 1280×800, 19/10/2026):

| Cenário | Processos | CPU | Privado | Working set |
|---|---|---|---|---|
| Cliente Electron, tocando (referência) | 6 | 16–32% | 672 MB | 1009 MB |
| WebView2 vazio (about:blank) | 8 | 0,3% | 121 MB | 319 MB |
| Site parado na home | 9 | 1,5% | 380 MB | 638 MB |
| **Site tocando** | 10 | 2,7% | 452 MB | 718 MB |
| **Tocando, na bandeja** (escondido + memória Low) | 10 | 2,9% | 473 MB | **242 MB** |
| Tocando sem GPU (`--disable-gpu`) | 10 | 15% | 317 MB | 644 MB |
| Tocando com V8 lite (`--js-flags=--lite-mode`) | 10 | 4,5% | 424 MB | 695 MB |

- **Privado** é a memória só deste app; **working set** é o que está na RAM agora, incluindo memória compartilhada.
- Na bandeja, o WebView2 manda o renderer para o arquivo de paginação: o privado fica, a RAM física cai de 718 para 242 MB e a música continua.
- Sem GPU e V8 lite trocam RAM por CPU. **Os dois ficam de fora.**
- De onde vem a RAM (tocando, privado): processo principal 40 MB + 3,5 MB do nosso exe Rust, GPU 188 MB, renderer 183 MB, rede/áudio/storage 33 MB. O ganho sobre o Electron vem de tirar o Node (−110 MB) e os plugins JS dentro da página. O que sobra é o Chromium (~120 MB fixos) mais a própria página, e isso nenhum wrapper corta.

### Metas

| | Cliente Electron | feathertune |
|---|---|---|
| RAM tocando, janela aberta | ~670 MB privado / ~1 GB WS | **≤ 480 MB privado / ≤ 750 MB WS** |
| RAM na bandeja | igual ao aberto | **≤ 260 MB WS** |
| CPU tocando | 16–32% de um núcleo | **≤ 4%** |
| Instalador | ~100 MB (Chromium embutido) | **poucos MB** (o WebView2 já vem no Windows) |
| Abertura | 4,5 s até o conteúdo | **2,0 s até a barra do player** (Fase 1, cache quente) |

### Medições do app

`measure.ps1` na raiz do repo, logado, janela 1280×800, uma faixa fixa. A saída vai para `results/` (fora do Git: tem caminhos locais e histórico de músicas).

| Fase | Data | Cenário | Processos | CPU | Privado | WS | Abertura até a barra |
|---|---|---|---|---|---|---|---|
| 1 | 19/10/2026 | Tocando, janela aberta | 9 | **2,8%** | **408 MB** | 697 MB | 2,0 s |
| 1 | 19/10/2026 | Tocando, na bandeja (fechou aos 25 s, memória Low) | 9 | 2,4% | 418 MB | **167 MB** | 1,9 s |
| 1 | 19/10/2026 | Primeira abertura, perfil novo (cache frio) | 9 | — | — | — | 6,4 s |
| 1 | — | 1 h na bandeja | pendente | | | | |
| 2 | 19/10/2026 | Tocando, janela aberta (3 rodadas, mediana) | 9 | **3,6%** (3,1–3,9%) | **408 MB** | 711 MB | 2,0 s |
| 2 | 19/10/2026 | Tocando, ponte mandando 1×/s (descartado; 3 rodadas, mediana) | 9 | 6,2% (3,9–6,5%) | 412 MB | 720 MB | 2,0 s |
| 3 | 20/10/2026 | Tocando, janela aberta, integrações no padrão (3 rodadas, mediana) | 9 | 3,6% (2,4–4,7%) | 407 MB | 721 MB | 2,0 s |
| 3 | 20/10/2026 | Tocando, na bandeja, integrações no padrão (3 rodadas, mediana) | 9 | 3,4% (3,2–3,7%) | 415 MB | 185 MB | 2,0 s |
| 4 | 20/10/2026 | `--profile dist`, experimentos mantidos ligados, janela aberta (3 rodadas, mediana) | 9 | **3,3%** (3,1–3,4%) | **403 MB** | 733 MB | 1,9 s |
| 4 | 20/10/2026 | `--profile dist`, experimentos mantidos ligados, na bandeja (3 rodadas, mediana) | 9 | 3,3% (2,4–3,4%) | 411 MB | **188 MB** | 2,0 s |
| 5 | 20/10/2026 | `--profile dist`, casca nativa, plugins no padrão, janela aberta (3 rodadas, mediana) | 9 | 4,5% (4,1–5,5%) | 430 MB | 761 MB | 1,9 s |
| 5 | 20/10/2026 | `--profile dist`, casca nativa, na bandeja (3 rodadas, mediana) | 9 | 3,6% (3,4–3,9%) | 436 MB | 217 MB | 1,9 s |
| 5 | 20/10/2026 | `--profile dist`, mini player aberto, janela principal escondida (3 rodadas, mediana) | 9 | 4,0% (3,2–5,3%) | 442 MB | 218 MB | 2,0 s |

- Linhas da Fase 5: todas ficaram acima das linhas dist da Fase 4 (+27 MB privados e +1,2 ponto de CPU com a janela aberta, +29 MB de WS na bandeja), além da variação delas. Não houve base no mesmo dia (medir de novo o build da Fase 4 no mesmo dia ficou de fora para poupar tempo), então ainda não se sabe quanto vem da casca e quanto do site ou do runtime do WebView2, que podem ter mudado desde a manhã; veja os itens em aberto da Fase 5.
- As linhas da Fase 3 são as rodadas da base da Fase 4 (E0): mesmo binário, atalhos e notificação de faixa ligados, Discord e Last.fm desligados porque a máquina de teste não tem id de aplicativo nem chave de API configurados; essas duas integrações **não** estão nos números. Linhas da Fase 4: veja os experimentos abaixo.
- As Fases 1 e 2 foram medidas com o perfil release antigo (LTO thin, 1 codegen unit). O processo do exe ocupa ~4 MB privados de qualquer jeito, então o perfil não mexe nesses números; a tabela final da Fase 4 usa `--profile dist`.
- Na bandeja o `<video>` seguiu avançando e a sessão continuou logada.
- Fase 2: cada chamada da ponte é um fetch com preflight CORS. Mandar a posição 1×/s custava ~3% de CPU (A/B no mesmo build: 3,2% sem, 6,2% com). Por isso a página manda só na troca de faixa, play/pause e seek, mais uma correção a cada 15 s, e o Rust extrapola a posição (`State::position_now()`).

## Arquitetura

```
feathertune.exe (Rust, Tauri 2)                      WebView2 (o Chromium do sistema)
├─ bandeja: tocar/pausar/próxima/sair                └─ o site oficial, intacto
├─ fechar = ir pra bandeja (memória Low)                 ├─ script injetado: lê o estado do player
├─ 1 instância, autostart, atalhos globais               │   (navigator.mediaSession + <video>)
├─ estado do player  ◄──── ponte de mão única ───────────┘   e manda só isso pro Rust
├─ teclas de mídia / flyout do Windows (SMTC)
├─ Discord Rich Presence, Last.fm, notificação de faixa
├─ filtro de rastreadores (só listas de privacidade, Fase 4)
├─ barra de título e mini player nativos (Direct2D, Fase 5)
└─ janela de configurações: outra webview, página nossa
   num esquema próprio, sem acesso ao site (Fase 5)
```

- **Rust:** tudo do Windows (bandeja, SMTC, atalhos, autostart, notificações), as integrações, o controle de memória (escondeu → `MemoryUsageTargetLevel=Low`, mostrou → `Normal`) e o filtro de rede.
- **Página:** o site como é entregue. Login, biblioteca, letra, rádio e recomendações já vêm prontos; o app não reimplementa nada disso.
- **Ponte:** um script injetado (`initialization_script`) observa o player e manda ao Rust um JSON com título, artista, capa, tempo e estado. O Rust trata como dado não confiável: valida e não executa nada. Os comandos Rust → página (pausar, próxima) clicam nos botões do player; se o site mudar a interface, quebram uns 3 seletores CSS, não um protocolo.
- **Segurança:** a página remota não recebe nenhum comando do Tauri além do relato de estado, e a janela de configurações só alcança os próprios comandos (veja a Fase 5); a navegação fica presa aos domínios do site e do login (o resto abre no navegador padrão).

## Riscos

1. **Login dentro do WebView2:** funciona (19/10/2026). Provedores bloqueiam login embutido principalmente por user agent falso ou flags de automação, então o app nunca usa nenhum dos dois.
2. **Runtime compartilhado:** o WebView2 é atualizado pelo Windows. Bom para segurança e codecs, mas uma atualização pode mudar o consumo sem mudança nossa.
3. **Termos de uso:** embrulhar o site em que o usuário loga é o que um navegador faz; nenhuma política de API se aplica porque nenhuma API é usada. Bloquear ou alterar anúncio passaria desse limite, então nunca é feito.

## Fases

**Como executar:** abra uma sessão nova com o modelo indicado e peça "execute a Fase N de `docs/PLAN.pt-BR.md`" (prompts em [PROMPTS.pt-BR.md](PROMPTS.pt-BR.md)). Uma fase por sessão, em ordem. Toda fase termina com medição (`measure.ps1`, rodado pelo mantenedor no Windows) e o resultado anotado na tabela acima, nos dois arquivos do plano.

**Idioma:** todo o código em inglês (identificadores, comentários, nomes de arquivos, chaves do `config.toml`, logs, mensagens de commit). A documentação existe em inglês e em português do Brasil, sempre em sincronia.

**Divisão de modelos:** Opus 5.5 (medium) para interop COM/WinRT com o WebView2 e o Windows, segurança da página remota e a arquitetura que as outras fases herdam. Sonnet 5.5 (high) para o que é bem especificado: plugins oficiais do Tauri, APIs HTTP públicas e experimentos guiados por medição.

| Fase | Modelo | Por quê |
|---|---|---|
| 1. Casca | Opus 5.5 medium | estrutura, capabilities da página remota, interop COM de memória, o medidor comum |
| 2. Ponte e teclas de mídia | Opus 5.5 medium | canal página → Rust com dado não confiável, SMTC via WinRT |
| 3. Integrações | Sonnet 5.5 high | plugins oficiais e APIs públicas sobre um estado pronto |
| 4. Dieta e entrega | Sonnet 5.5 high | experimentos medidos, instalador |
| 5. Casca nativa | Opus 5.5 medium | interop Win32/Direct2D da janela, multiwebview, a capability de uma janela local ao lado da página não confiável, o modelo de plugins que a Fase 6 herda |
| 6. Visual | Sonnet 5.5 high | módulos de injeção e uma API HTTP pública sobre a casca da Fase 5, medidos ligados e desligados |

### Fase 1: Casca (concluída em 19/10/2026)

App Tauri 2.12 em `app/`, sem frontend próprio: a janela principal abre o site. Perfil do WebView2 em `%LOCALAPPDATA%\feathertune`. Flags do Chromium num único `--disable-features` (o Chromium só respeita o último). Navegação presa aos domínios do site e do login. Fechar esconde na bandeja com `MemoryUsageTargetLevel=Low`. Bandeja: mostrar/esconder, tocar/pausar, próxima, sair.

**Injeções como módulos** (semente dos plugins futuros): todo JS/CSS que entra na página fica em `app/injections/<id>/` (`page.js` e/ou `style.css`), tem liga/desliga em `[injections]` no `config.toml` do perfil e é montado num único `initialization_script`, cada módulo isolado num `try/catch` próprio.

Resultado: tocando 408 MB privados e 2,8% de CPU, bandeja 167 MB de WS, abertura até a barra do player 2,0 s, o login persiste ao reabrir. **Pendente:** o teste de 1 h na bandeja (`./measure.ps1 -Name tray-1h -ArgumentList $a -HideAfter 25 -Minutes 60`, sem abrir o app durante a hora; passa se o WS final ≤ WS aos 5 min + 10%).

### Fase 2: Ponte e teclas de mídia (concluída em 20/10/2026)

- Injeção `core` (sempre ligada, só no frame principal) → comando único `player_state` → `Player` (`app/src/player.rs`, API documentada no topo). Capability `app/capabilities/player-bridge.json`: só esse comando, só a janela `main`, só a origem do site e nada para origem local. O `frontendDist` saiu do `tauri.conf.json`: com ele, o Tauri tratava o site remoto como origem *local*, e qualquer capability local futura vazaria para a página.
- O SMTC do próprio WebView2 aparecia como `msedgewebview2.exe`, então foi desligado (`HardwareMediaKeyHandling`) e o app publica o seu (`app/src/smtc.rs`, preso à janela principal).
- **Nome no flyout:** um exe solto aparece como "Unknown app". O que resolve é o processo ter um AppUserModelID (`dev.feathertune.desktop`) e existir um atalho no Menu Iniciar com o mesmo ID. O app cria `%APPDATA%\Microsoft\Windows\Start Menu\Programs\feathertune.lnk` se não existir; o instalador da Fase 4 o cria com o mesmo AUMID e o desinstalador o remove (veja a Fase 4).
- Teclas de mídia com o app na bandeja: tocar/pausar ≤ 16 ms, próxima ~0,5 s, anterior ~0,2 s até o novo estado chegar no Rust. CPU tocando: mediana de 3,6%, 408 MB privados.

### Fase 3: Integrações (Sonnet 5.5 high) — conferida no Windows em 20/10/2026; **não concluída, 3 itens em aberto** (veja o fim desta seção)

Consome só o estado da Fase 2. Não mexe na ponte, no WebView nem nas flags.

- Plugins oficiais: `single-instance` (abrir de novo mostra a janela existente), `autostart` (opção no menu da bandeja, desligada por padrão), `window-state`.
- Notificação de troca de faixa, só com a janela escondida.
- Atalhos globais (`tauri-plugin-global-shortcut`): tocar/pausar, próxima, anterior. As teclas ficam no `config.toml` do perfil.
- Discord Rich Presence (crate `discord-rich-presence`): título, artista e tempo; limpa ao pausar; falha em silêncio e reconecta se o Discord estiver fechado.
- Last.fm:
  - chave de API do próprio usuário, lida de configuração local, nunca no código nem no Git;
  - autenticação desktop (`auth.getToken` → navegador → `auth.getSession`), session key no Windows Credential Manager (crate `keyring`);
  - `track.updateNowPlaying` ao começar a faixa; `track.scrobble` quando a faixa tem mais de 30 s e já tocou metade dela ou 4 min, o que vier primeiro;
  - fila offline simples para scrobbles que falharem.
- **Pronto quando:** cada item funciona com o app na bandeja, o mantenedor confirma o Discord e o scrobble num perfil real do Last.fm, e a CPU tocando continua ≤ 4%.

Notas da implementação (`app/src/{settings,shortcuts,notify,discord,lastfm}.rs`; cada integração é uma thread parada em `Player::subscribe()`, sem polling):

- Tudo vem do `config.toml` do perfil (`[shortcuts]`, `[notifications]`, `[discord]`, `[lastfm]`; veja o README). O id do aplicativo do Discord e a chave e o secret do Last.fm são do próprio mantenedor e ficam só ali, fora do Git. Valor vazio desliga a integração.
- **Notificação sem o plugin:** o `tauri-plugin-notification` injeta em todo webview um script que troca o `window.Notification` da página, o que quebra "a página não é confiável e não mexemos nela". O toast sai do Rust com `tauri-winrt-notification` (a crate que o plugin usa), sob o AppUserModelID do app.
- **Autostart** passa `--hidden`, então a partida no login vai direto para a bandeja (janela criada invisível, memória Low). **Window-state** salva só tamanho, posição e maximizado (nunca a visibilidade) e fica desligado com `--measure`, para a medição manter o 1280×800 fixo.
- **Atalhos padrão:** `Ctrl+Alt+Space`, `Ctrl+Alt+Right`, `Ctrl+Alt+Left`; `""` desliga um; combinação que outro app já usa é registrada no log e ignorada.
- **Discord:** atividade "Listening" (título, artista, capa, barra de tempo a partir da posição extrapolada); enviada na troca de faixa e ao retomar, e de novo só se a posição derivar > 2 s (um seek); limpa ao pausar. Com o Discord fechado, a conexão é tentada de novo no próximo evento do player.
- **Last.fm:** o item da bandeja conecta ou desconecta. O login abre o navegador e consulta `auth.getSession` a cada 5 s por 5 min; a session key vai para o Credential Manager (serviço `feathertune`). O ponto do scrobble conta só tempo tocado (pausas fora) e um timer acorda a thread nesse momento. Tocar a mesma faixa de novo (a posição volta depois do scrobble) conta como nova audição. Scrobbles que falham esperam em `lastfm-queue.json` no perfil (lotes de 10, nova tentativa a cada 5 min, na partida e no próximo scrobble; itens com mais de 14 dias são descartados porque o Last.fm os ignora). Session key rejeitada (erro 9) descarta a sessão e pede novo login.
- **Limites da sessão na nuvem:** `cargo clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings` e `cargo fmt --check` passaram no Linux; o app não foi executado lá.
- **Executado no Windows (20/10/2026, build release, perfil logado, sem supervisão).** A bandeja foi acionada postando o callback do ícone na janela da bandeja do app e as teclas do menu no menu popup (sem mouse de verdade); os atalhos, com teclas injetadas; o estado, lido do log do `--measure`.
  - Bandeja: Mostrar/ocultar (e clique esquerdo) esconde na bandeja com memória Low e mostra de novo com Normal; Tocar/pausar pausa e retoma o `<video>`; Próxima troca a faixa. "Start with Windows" inverte a marca e o estado se mantém após reiniciar; a chave Run em si não pôde ser lida do shell de teste, então a entrada no registro **não está confirmada** (desligado de novo no fim). O item do Last.fm fica desabilitado com "Last.fm: add the keys to config.toml", como projetado.
  - Atalhos: `Ctrl+Alt+Right` (próxima) e `Ctrl+Alt+Left` (reinicia a faixa) funcionam. `Ctrl+Alt+Space` pertence a outro app na máquina de teste, então é registrado no log e ignorado, como projetado; o tocar/pausar foi conferido com `play_pause = "Ctrl+Alt+P"` e funciona.
  - Toast de troca de faixa com a janela escondida: o código rodou sem erro, mas o toast em si não foi visto.
  - Medição: as duas linhas da Fase 3 acima (CPU 3,6% / 3,4%, dentro da meta ≤ 4%).
  - **Ainda em aberto:** (1) Discord Rich Presence, não executado: sem id de aplicativo no `config.toml` e o Discord não estava aberto; (2) login e scrobble de verdade no Last.fm, não executados: sem chave de API nem sessão na máquina (`cmdkey /list` não tem entrada do Last.fm); (3) a entrada na chave Run do "Start with Windows" e o toast visível. Troque a nota do título por "Fase 3: concluída em <data>" quando (1) e (2) forem confirmados num perfil real.

### Fase 4: Dieta e entrega (Sonnet 5.5 high) — medida e instalada no Windows em 20/10/2026; **não concluída, 3 itens em aberto** (veja o fim desta seção)

Os experimentos, a medição final do dist e a conferência de instalação/desinstalação silenciosa rodaram no Windows em 20/10/2026. Em aberto: a coluna Electron da tabela final (nenhum cliente Electron instalado), o caminho "excluir dados" da desinstalação e os itens da Fase 3 acima. Troque a nota do título por "Fase 4: concluída em <data>" quando forem fechados.

**Experimentos** (um por vez, cada um medido com `measure.ps1`, 3 rodadas, mediana; só fica o que mostrar ganho). Todos os interruptores estão no `config.toml` do perfil, então a tabela inteira precisa de **um** build. Os interruptores testados (`performance` era um módulo CSS, hoje apagado):

```toml
[injections]
performance = true                 # (apagado) CSS: sem animação, transição, rolagem suave nem backdrop-filter
[browser]
disable_features = ["Nome1", "Nome2"]   # entram no único --disable-features (nomes: letras, dígitos, _)
[privacy]
tracker_filter = true              # o filtro de rastreadores abaixo
```

```powershell
cd app; cargo build --release; cd ..                     # uma vez para a fase toda
$a = '--measure', '<url da faixa>'
1..3 | % { ./measure.ps1 -Name base-open-$_ -ArgumentList $a }                         # janela aberta
./median.ps1 -Prefix base-open-
1..3 | % { ./measure.ps1 -Name base-tray-$_ -ArgumentList $a -HideAfter 25 -Wait 50 }  # na bandeja
./median.ps1 -Prefix base-tray-
```

Repita com outro prefixo (`e1-open-`, `e1-tray-`, ...) depois de mudar só a linha daquele experimento no `config.toml`, e ponha as duas saídas do `median.ps1` na tabela. **Regra:** a mudança só fica se a mediana superar a da base por mais que a própria variação min–max da base *e* o login e a reprodução continuarem funcionando. Um nome de feature que o Chromium não conhece é ignorado sem erro, então "sem mudança" também pode significar que o nome não existe no runtime instalado. Depois dos experimentos isolados, meça uma vez os que ficaram juntos.

Como a regra foi aplicada (build release, faixa fixa, logado, toda rodada conferida com `loggedIn: true`, `paused: false` e tempo do `<video>` ≥ 40 s no fim; as 54 rodadas passaram): por métrica, a variação da base é CPU aberto 2,3 pontos (2,4–4,7%), privado 9 MB (405–414), WS 10 MB (715–725), WS na bandeja 2 MB (184–186). Um experimento fica se pelo menos uma métrica melhora por mais que a sua variação e nenhuma piora por mais que a sua. As variações são largas perto dos ganhos, então as duas mudanças mantidas são marginais; desfazer cada uma é uma linha.

| # | Experimento (interruptor) | Aberto: CPU / privado / WS | Bandeja: WS | Decisão |
|---|---|---|---|---|
| E0 | Base: app atual, tudo abaixo desligado | 3,6% (2,4–4,7) / 407 MB / 721 MB | 185 MB | — |
| E1 | Módulo CSS `performance` (`performance = true`) | 4,0% / 420 MB / 717 MB | 185 MB | não ficou: privado +13 MB (> variação), CPU sem melhora; módulo apagado |
| E2 | `SpareRendererForSitePerProcess`: sem processo renderer reserva | 4,6% (4,6–13,2) / 424 MB / 728 MB | 187 MB | não ficou: pior em todas as métricas |
| E3 | `Prerender2`: sem pré-renderização de páginas | 4,7% / 410 MB / 713 MB | 185 MB | não ficou: WS −8 MB está dentro da variação, CPU pior |
| E4 | `MediaRouter`, `DialMediaRouteProvider`: sem descoberta de Cast | 4,0% / 408 MB / 709 MB | 185 MB | **ficou**: WS −12 MB (> 10), nada pior |
| E5 | `OptimizationHints`, `OptimizationGuideModelDownloading`, `OptimizationHintsFetching`, `OptimizationTargetPrediction`: sem hints nem download de modelos | 4,2% / 409 MB / 712 MB | 185 MB | não ficou: WS −9 MB está dentro da variação |
| E6 | Filtro de rastreadores, lista embutida (`tracker_filter = true`) | 2,9% (2,9–3,2) / 406 MB / 705 MB | 185 MB | **ficou**: WS −16 MB (> 10), CPU menor; bloqueou um host (`www.googletagmanager.com`), reprodução intacta |
| E7 | Filtro + lista de privacidade em `privacy-list.txt` (EasyPrivacy, 1,5 MB) | 3,5% / 408 MB / 710 MB | 188 MB | não ficou: WS na bandeja +3 MB (> 2) e bloqueou o mesmo único host do E6 |
| E8 | E4 + E6 juntos | 4,3% (3,8–4,5) / 409 MB / 712 MB (uma rodada teve 11 processos, 486 MB privados) | 185 MB | — (o ganho combinado não se confirma: WS −9 MB está dentro da variação) |

Nas rodadas da bandeja (E0–E8) a CPU das medianas ficou entre 2,8% e 3,7%. Os interruptores mantidos são o padrão agora (`DISABLED_FEATURES` em `app/src/main.rs`, padrão de `tracker_filter` em `app/src/settings.rs`); o que vale é o número da rodada final `--profile dist` abaixo.

Consideradas e **não** testadas, de propósito: isolamento de sites desligado (`site-per-process`, enfraquece o isolamento de uma página não confiável); `--disable-component-update` (pode congelar os componentes de mídia do runtime e quebrar a reprodução); features do Privacy Sandbox ou ligadas a anúncios (mexem em anúncios); `--disable-gpu` e V8 lite (já descartados, veja Peso medido); `CalculateNativeWinOcclusion` (a bandeja já esconde a view de forma explícita, e essa faria janelas escondidas contarem como visíveis). Uma feature que fica sai de `disable_features` e vai para `DISABLED_FEATURES` em `app/src/main.rs`; um módulo ou experimento que não fica é apagado, e a linha aqui guarda os números.

**Módulo `performance` (apagado):** zerava duração e atraso de animações e transições, `scroll-behavior: auto` e `backdrop-filter: none` como CSS injetado. O E1 não mostrou ganho (mais memória privada, CPU sem queda), então `app/injections/performance/` e o interruptor saíram.

**Filtro de rastreadores** (`app/src/privacy.rs`, `[privacy] tracker_filter`, **ligado por padrão** desde o E6; `tracker_filter = false` desliga): `WebResourceRequested` do WebView2 via `with_webview` + crate `adblock` (0.13, features padrão) com as regras cosméticas descartadas, então só consegue bloquear requisições, nunca mudar a página. Feito para não conseguir tocar em anúncios nem na reprodução, no código e não confiando na lista:

- o handler só é registrado para scripts, imagens, XHR/fetch, pings e "other"; documentos, frames, mídia, fontes e folhas de estilo nunca chegam nele;
- hosts dos domínios do player, qualquer domínio Google e os domínios de entrega de anúncios do Google (doubleclick, googlesyndication, googleadservices, 2mdn, ...) são pulados antes de consultar a lista, então mesmo uma lista de anúncios posta por engano não muda nada ali;
- a lista embutida (`app/filters/privacy.txt`, escrita para este projeto) tem só hosts de analytics e gravação de sessão de terceiros. Mais regras vão em `privacy-list.txt` no perfil (só listas de privacidade: decisão do mantenedor, e o E7 mede o custo de uma completa);
- a requisição bloqueada recebe um 403 vazio. Com `--measure`, cada host bloqueado é impresso uma vez (`tracker_blocked=<host>`), então uma rodada mostra o que foi pego. Se nada for bloqueado no site, o filtro não tem ganho e é candidato a sair, qualquer que seja o custo por requisição.
- Situação: executado no E6, E7, E8 e nas rodadas finais do dist. Neste site bloqueou um host, `www.googletagmanager.com`, em todas as rodadas; login e reprodução não foram afetados e a CPU não subiu. A EasyPrivacy (E7) não pegou mais nada. O handler roda na thread de UI a cada subrecurso desses tipos; nenhuma trava foi vista.

**Instalador** (`app/tauri.conf.json` → `bundle`, `app/installer-hooks.nsh`): bundler NSIS do Tauri, por usuário (sem admin; instala em `%LOCALAPPDATA%\feathertune`, **a mesma pasta do perfil**: conferido em 20/10/2026), WebView2 `downloadBootstrapper` (não aumenta o instalador; baixa o bootstrapper só onde falta o runtime, o que não é o caso do Windows 11 nem de um Windows 10 atualizado; `embedBootstrapper` somaria ~1,8 MB para instalação offline), inglês e português do Brasil conforme o idioma do sistema. O CI gera com `build -- --profile dist --locked` (job `dist`; artefato `feathertune-<sha>`, instalador em `bundle/nsis/*-setup.exe`). À mão: `cd app; npx @tauri-apps/cli@2.12.3 build -- --profile dist`.

- **Atalho e AUMID:** o template padrão cria `Start Menu\Programs\feathertune.lnk` e grava nele o identificador do bundle como AppUserModelID, que é `dev.feathertune.desktop`, o ID que o `smtc.rs` põe no processo. É o mesmo caminho que o app escreve quando o atalho não existe, então não duplica, e o desinstalador o remove (quando ainda aponta para o exe instalado), além da jump list e da entrada "Iniciar com o Windows".
- **Perfil:** o checkbox "excluir dados do aplicativo" do template limpa pastas com o nome do identificador do bundle, não `%LOCALAPPDATA%\feathertune`; por isso o `installer-hooks.nsh` limpa o perfil de verdade (login, `config.toml`, fila do Last.fm) e a credencial do Last.fm (`cmdkey /delete:lastfm-session.feathertune`, nome do alvo deduzido do código da crate `keyring`, **a confirmar**) só quando a caixa está marcada. Desmarcada, nada do usuário é removido.
- **Conferência de instalação** ("instala e desinstala limpo"), no Windows. Resultado da rodada de 20/10/2026, com o instalador `dist` (2,98 MiB), **só em modo silencioso** (`/S`; a caixa não dá para marcar assim):
  - passo 1: instala (exit 0) em `%LOCALAPPDATA%\feathertune`, junto do perfil; o atalho do Menu Iniciar é gravado com o AUMID `dev.feathertune.desktop` (lido de volta por `IShellLink`/`IPropertyStore`; guardado como `VT_BSTR`, enquanto o atalho do próprio app usa `VT_LPWSTR`); o exe instalado abre, o perfil existente está logado (`loggedIn: true`) e a barra do player aparece em 1,9 s. O nome no flyout de mídia em si não foi olhado;
  - passo 2: instalar em silêncio por cima do app aberto fecha o app, sai com 0 e o perfil sobrevive;
  - passo 3 (desmarcado): exe, `uninstall.exe`, atalho e entrada de desinstalação somem; `EBWebView` e `config.toml` continuam. Não havia entrada Run (o autostart estava desligado);
  - passo 4 (marcado): **não executado.** Pela leitura do `installer-hooks.nsh`: com a caixa marcada (e fora do modo de atualização) remove `%LOCALAPPDATA%\feathertune` (perfil e pasta de instalação são a mesma) e roda `cmdkey /delete:lastfm-session.feathertune`. O nome do alvo bate com o padrão da crate `keyring` (`<user>.<service>` = `lastfm-session.feathertune`, lido do código-fonte); não havia credencial do Last.fm para comparar com o `cmdkey /list`.
- O roteiro original, para o caminho da caixa:
  1. instalar e abrir pelo Menu Iniciar: a janela abre, o login funciona (ou o perfil logado vale), o flyout de mídia mostra o nome do app, o atalho tem o AUMID (`dev.feathertune.desktop`);
  2. instalar de novo por cima do app aberto: pede para fechá-lo, o perfil sobrevive;
  3. desinstalar **desmarcado**: pasta de instalação, atalho e entrada Run somem; `%LOCALAPPDATA%\feathertune` continua; reinstalar mantém o login;
  4. desinstalar **marcado**: a pasta do perfil some, e `cmdkey /list | findstr lastfm` volta vazio.

**Tabela final: cliente Electron × feathertune.** Build `--profile dist`, 3 rodadas, mediana, mesmo critério acima. Electron: `./measure.ps1 -Name electron-open-$_ -Exe <exe do cliente> -ArgumentList <args dele>` com a mesma faixa tocando; feathertune: `-Exe app/target/dist/feathertune.exe`. Preencha com a saída do `median.ps1`.

| | Cliente Electron | feathertune (dist) | Meta |
|---|---|---|---|
| Tocando, janela aberta: CPU | não medido: nenhum cliente Electron instalado (tabela do protótipo: 16–32%) | **3,3%** (3,1–3,4) | ≤ 4% ✔ |
| Tocando, janela aberta: privado | não medido (antes: 672 MB) | **403 MB** (399–411) | ≤ 480 MB ✔ |
| Tocando, janela aberta: WS | não medido (antes: 1009 MB) | **733 MB** (732–740) | ≤ 750 MB ✔ |
| Tocando, na bandeja: WS | não medido | **188 MB** (188–190) | ≤ 260 MB ✔ |
| Processos | não medido (antes: 6) | 9 | — |
| Abertura até a barra do player / conteúdo | não medido (antes: 4,5 s) | mediana 1,9 s (1,9–2,9; uma rodada mais lenta) | ≤ 2,0 s ✔ |
| Tamanho do instalador | não medido (antes: ~100 MB) | 2,98 MiB | poucos MB ✔ |

Coluna feathertune: `app/target/dist/feathertune.exe`, experimentos mantidos ligados, perfil logado, uma faixa fixa, 3 rodadas cada (aberto e bandeja), todas conferidas como tocando. Os valores "antes" da coluna Electron são os números da época do protótipo, das tabelas acima, e não uma medição nova; o critério difere em detalhes (outro dia, faixa e versão do cliente).

**Limites da sessão na nuvem (histórico) e o que rodou no Windows:** a sessão na nuvem só rodou `cargo clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings` e `cargo fmt --check` no Linux; o job `dist` do CI gera o instalador no Windows. Em 20/10/2026 uma sessão sem supervisão no Windows rodou: o build release, as conferências de bandeja/atalhos da Fase 3 listadas lá, a base e E1–E8 (3 rodadas abertas + 3 na bandeja cada), o build e a medição `--profile dist` e a conferência silenciosa de instalar/reinstalar por cima/desinstalar. Pendente para o mantenedor: Discord e login/scrobble reais no Last.fm; a confirmação da chave Run e do toast; a coluna de comparação com o Electron; o caminho "excluir dados" da desinstalação; e as notas dos títulos das Fases 3 e 4.

### Fase 5: Casca nativa (Opus 5.5 medium) — feita e medida no Windows em 20/10/2026; **não concluída, 3 itens em aberto** (veja o fim desta seção)

O app ganha visual e controles próprios em volta do site: barra de título, uma janela de configurações que lista os plugins, um mini player e Last.fm sem digitar chaves.

**Por que não uma engine própria** (avaliado em 20/10/2026): o grosso do peso é a própria página (WebView2 vazio 121 MB privados, site tocando ~450 MB), e qualquer engine que rode o site oficial carrega isso. CEF e Electron trazem o próprio Chromium (mais pesados, e 200+ MB a mais no instalador); o Servo só começou em junho de 2026 o trabalho com Media Source Extensions, de que o player precisa; não existe Gecko embutível; um cliente nativo exigiria a API privada (fora, veja as regras). O WebView2 fica, e esta fase muda o que está em volta dele.

- **Barra de título:** a janela perde a moldura do sistema (`decorations(false)`) e o Rust desenha uma faixa no topo com Direct2D/DirectWrite: nome do app, botão de menu, minimizar, maximizar/restaurar, fechar. O site passa para uma webview filha posicionada abaixo da faixa (multiwebview do Tauri: `Window` + `add_child`, feature `unstable`). O hit-test é nativo (`WM_NCHITTEST`: a faixa arrasta, `HTMAXBUTTON` no maximizar para aparecerem os snap layouts do Windows 11, bordas de redimensionar). A faixa respeita o DPI, segue claro/escuro e uma cor de tema (a Fase 6 define pela capa). A página remota não ganha permissão nova, porque a faixa não é HTML. **Plano reserva** se o multiwebview travar (foco, teclado, memória Low ou SMTC quebrando na webview filha): manter a moldura do sistema colorida pelo DWM (`DWMWA_CAPTION_COLOR`, `DWMWA_TEXT_COLOR`, modo escuro), com o menu na bandeja e num atalho, e escrever o motivo nesta seção.
- **Janela de configurações:** o botão de menu abre uma janela separada com HTML/CSS nosso: plugins liga/desliga, conectar/desconectar Last.fm, Discord, atalhos, notificações, privacidade, iniciar com o Windows, mini player. Ela é criada ao abrir e destruída ao fechar, então não custa nada fechada. Carrega de um esquema de URI próprio, não do `frontendDist` (a Fase 2 o removeu porque fazia o site remoto contar como origem local). Tem capability própria, limitada a essa janela e a esse esquema. A capability do site não muda: só `player_state`, só a webview do site. As mudanças vão para o `config.toml` mantendo os comentários do usuário (pode entrar o `toml_edit` para isso, se o `toml` não der). Valem na hora quando possível; senão a janela avisa que precisa reiniciar.
- **Plugins:** isto formaliza o objetivo de "plugins que você liga e desliga". Todo recurso opcional (um módulo de injeção, uma integração em Rust, ou os dois) é um plugin com id, nome, descrição, custo medido e um interruptor no `config.toml`, e a janela de configurações lista todos. Sem carregar DLL nem código baixado. A bandeja mantém os itens rápidos.
- **Chave do Last.fm no build:** a API key e o secret vêm de dois secrets do repositório (`FEATHERTUNE_LASTFM_API_KEY`, `FEATHERTUNE_LASTFM_API_SECRET`) para o build `dist` do CI via `option_env!`. Valores no `config.toml` continuam tendo prioridade, e um build sem eles (forks, builds locais) funciona como hoje. Nunca vão para o Git. Dá para extraí-los do exe, como em todo cliente desktop; a session key continua no Gerenciador de Credenciais. **Mantenedor:** criar os dois secrets.
- **Mini player:** uma janela nativa pequena, sempre no topo (Direct2D), com capa, título, artista, progresso e anterior/tocar-pausar/próxima. É alimentada só pelo `Player`, e os comandos vão pelo caminho Rust → página que já existe. Abrir o mini player esconde a janela principal (memória Low, como na bandeja). O Rust baixa a URL da capa que vem no estado, só dos hosts de imagem que o site usa, e decodifica com WIC (crate `windows`). Liga e desliga pelo menu da barra, pela bandeja e por um atalho opcional.
- **Custo:** nenhum crate novo, fora o `toml_edit` se precisar. Features novas do `windows` (Direct2D, DirectWrite, WIC, GDI) e a feature `unstable` do Tauri são esperadas. Medir como na Fase 4 (3 rodadas, mediana; o `--measure` mantém 1280×800 para a janela inteira).
- **Pronto quando:**
  - a barra arrasta, faz snap, redimensiona, maximiza, restaura e fecha como uma janela normal, também em 150% de escala e num segundo monitor;
  - fechar continua indo para a bandeja com memória Low, e teclas de mídia e o flyout continuam funcionando;
  - a janela de configurações muda todas as opções, e fechá-la não deixa processo extra;
  - o Last.fm conecta num build do CI sem editar o `config.toml` (confirmado pelo mantenedor);
  - o mini player controla a reprodução com a janela principal escondida;
  - com a janela aberta fica dentro da variação da linha dist da Fase 4 (403 MB privados, 733 MB WS, 3,3% de CPU), a linha da bandeja não muda, e o mini player tem sua própria linha na tabela.

Notas de implementação (`app/src/{titlebar,gfx,miniplayer,plugins,settings_window}.rs`, `app/settings/`, `app/capabilities/`):

- **Janela:** `main` é uma `tauri::window::Window` sem webview própria (`decorations(false)`, sombra mantida) e o site é a webview filha `site`, 32 DIP abaixo do topo. A janela do `--measure` continua com 1280×800 de área cliente, então o site agora fica com 1280×768. O multiwebview não travou: memória Low ao esconder, filtro de rastreadores, SMTC e teclas de mídia, instância única, estado da janela e `--measure` funcionam na webview filha, então o plano reserva do DWM não foi usado.
- **Barra de título:** um subclass da janela `main` pinta a faixa (render target DC do Direct2D preso ao HDC do paint, então recorta como GDI e nunca desenha por cima da webview; texto em DirectWrite; glifos da "Segoe MDL2 Assets") e responde ao `WM_NCHITTEST`: bordas de redimensionar pelo tratamento padrão, depois `HTMINBUTTON`/`HTMAXBUTTON`/`HTCLOSE` nos botões (assim o Windows 11 mostra os snap layouts no maximizar), `HTCAPTION` no resto da faixa e `HTCLIENT` no botão de menu. Ela mantém os limites da webview no `WM_SIZE`, segue mudanças de DPI e o claro/escuro dos apps, e `gfx::set_theme_color` é o gancho da Fase 6. O visual é "tinta na água": quatro brilhos de cor suaves sobre tinta escura, sob um brilho de vidro, posicionados pelo id da faixa (cada faixa tem sua forma, que flui para a próxima em 0,7 s na troca; fora isso nada anima), com a faixa atual no centro da barra. O botão de menu abre um menu nativo (Settings, Mini player, Quit) que reaproveita os ids da bandeja. Fechar vai para a bandeja como antes.
- **Plugins** (`app/src/plugins.rs`, a lista única): `controls`, `tracker_filter`, `track_notifications`, `global_shortcuts`, `discord`, `lastfm`, `mini_player`, cada um com nome, descrição, custo (tirado das linhas desta tabela) e padrão (todos ligados). O interruptor é `[plugins] <id> = true|false`; as chaves antigas `[injections] controls`, `[privacy] tracker_filter` e `[notifications] track_change` continuam sendo lidas, e a janela de configurações as troca pela entrada em `[plugins]`. Atalhos, notificações e o mini player valem na hora; os outros avisam "restart". Um `config.toml` novo já sai com a lista de plugins comentada.
- **Janela de configurações:** `settings_window::open` cria a janela `settings` quando é aberta (mesmo perfil e mesmos argumentos do WebView2 do site, então ela divide o processo browser e só acrescenta um renderer) a partir de `feathertune://` (no Windows, `http://feathertune.localhost/`), com os arquivos compilados no exe a partir de `app/settings/`. É destruída ao fechar. As gravações passam pelo `toml_edit`, que mantém os comentários do usuário (o `toml` 1.x os descarta); um arquivo que não parseia não é gravado. As entradas são validadas no Rust: atalhos são parseados como atalhos, o id do Discord só aceita dígitos, a key e o secret do Last.fm só alfanuméricos, features do Chromium só nomes.
- **Como as duas capabilities ficam separadas:**
  - `player-bridge.json` agora é `"webviews": ["site"]` (era `"windows": ["main"]`, que também casaria com qualquer outra webview nessa janela), com `local: false` e só a origem do site, e concede só `allow-player-state`.
  - `settings-window.json` é `"webviews": ["settings"]`, `local: true`, e concede só os três comandos `settings_*`. O Tauri casa uma capability pelo rótulo da webview ou da janela **e** pela origem, então o site remoto (rótulo `site`, origem remota) não casa com a capability local nem com os comandos dela, e a página de configurações (rótulo `settings`, origem local) não casa com a ponte, que é só remota. Nenhuma das duas tem permissões `core:*`, então nenhuma página alcança as APIs de janela ou webview (eval, navegação, outras webviews).
  - Os comandos `settings_*` também conferem o rótulo da webview que chamou e recusam qualquer uma que não seja `settings`, e nenhum deles recebe uma webview nem executa script no site.
  - Esquemas próprios valem para todas as webviews (limitação do Tauri), então o handler do esquema responde 403 a qualquer pedido que não venha da webview `settings`, e só aceita GET. De qualquer forma, o site não consegue navegar até lá (a navegação dele continua presa aos domínios do site e do login).
  - A página de configurações tem uma CSP restrita (`default-src 'none'`, scripts e estilos só dela mesma) e monta o DOM só com `textContent`.
- **Last.fm:** o `lastfm.rs` lê `FEATHERTUNE_LASTFM_API_KEY`/`_SECRET` com `option_env!` no build; o `config.toml` vence se os dois valores dele estiverem preenchidos; sem nenhum dos dois, o Last.fm fica desligado como antes. O passo `dist` do CI passa os dois secrets como variáveis de ambiente (vazias em forks).
- **Mini player:** um popup Win32 (`WS_EX_TOPMOST`, janela de ferramenta, cantos arredondados do DWM, 340×96 DIP) desenhado com Direct2D, aberto pelo menu da barra, pela bandeja, por `[shortcuts] mini_player` ou por `feathertune.exe --mini-player`. Abrir esconde a janela principal com memória Low; "voltar" ou mostrar a janela principal o fecha. Uma thread em `Player::subscribe()` redesenha; quando a URL da capa muda, ela baixa a capa com `ureq` (só URLs que passam na mesma checagem de host do payload da página, sem redirecionamento, limite de 4 MB) e decodifica e reduz com WIC. Os avisos de faixa ficam de fora enquanto ele está aberto.
- **Correção achada no caminho:** o `ureq` era compilado com `native-tls-no-default`, mas o ureq 3.4 só aceita o provedor NativeTls com a feature `native-tls` ligada, e entra em pânico na primeira requisição https sem ela. Isso também teria quebrado toda chamada ao Last.fm desde a Fase 3 (nunca exercitada, sem chaves na máquina de teste). A feature agora é `native-tls`, o que traz mais um crate (`webpki-root-certs`, sem uso: as raízes continuam vindo do Windows pelo verificador da plataforma). É o único crate novo além do `toml_edit` (que já estava no `Cargo.lock` via `proc-macro-crate`).
- **Features novas do `windows`:** `Win32_Graphics_{Direct2D, Direct2D_Common, DirectWrite, Dwm, Dxgi_Common, Gdi, Imaging}`, `Win32_System_{LibraryLoader, Registry}`, `Win32_UI_{HiDpi, Input_KeyboardAndMouse, WindowsAndMessaging}`; feature `unstable` do Tauri.
- **Rodado no Windows (20/10/2026, build release, perfil logado, sem supervisão; mensagens de janela enviadas por script, sem mouse de verdade):**
  - o hit-test da faixa devolve CLIENT (menu), CAPTION, MINBUTTON, MAXBUTTON, CLOSE, TOP (borda) e CLIENT abaixo da faixa; a faixa pinta 0x202020 no modo escuro com glifo branco, e a página aparece abaixo dela;
  - WM_CLOSE esconde na bandeja com memória Low e a reprodução continua logada; uma segunda execução mostra a janela de novo;
  - a tecla de mídia tocar/pausar pausa e retoma pela SMTC;
  - o menu da barra abre a janela de configurações, que lista todos os plugins (lido por UI Automation); virar um interruptor gravou `[plugins] track_notifications` no `config.toml`; fechá-la remove o renderer dela (9 → 8 processos do WebView2);
  - `--mini-player` esconde a janela principal (memória Low), o mini player mostra a capa baixada, e o botão tocar/pausar dele pausa e retoma.
  - **Ainda em aberto:** (1) snap layouts, arrastar, redimensionar e maximizar/restaurar com mouse de verdade, em 150% de escala e num segundo monitor; digitar no site depois de trocar de janela (foco do teclado na webview filha) e a aparência do flyout; (2) o login do Last.fm num build do CI sem editar o `config.toml` (precisa dos dois secrets do repositório); (3) a medição: as linhas da Fase 5 ficaram acima das linhas dist da Fase 4, então meça o commit da Fase 4 (`73b93b2`) e este em seguida, no mesmo dia, para saber se a casca custa memória (se custar, olhe primeiro o segundo host do controller do WebView2 e o paint da barra). Troque a nota do título por "Fase 5: concluída em <data>" quando (1), (2) e (3) forem fechados.

### Fase 6: Visual (Sonnet 5.5 high) — planejada em 20/10/2026

Usa a casca da Fase 5 (barra de título, janela de configurações, plugins, decodificador WIC da capa). Cada item é um plugin, medido ligado e desligado com a regra da Fase 4 (só fica ligado por padrão se custar menos que a variação); o modo ambiente fica desligado por padrão qualquer que seja a medição.

- **Cor do álbum:** uma vez por faixa, o Rust tira a cor dominante da capa atual (decodificador da Fase 5, miniatura pequena) e aplica na barra de título, no mini player e na página, por uma variável CSS usada por um módulo de injeção (`app/injections/theme/`). Só apresentação, com contraste legível para o texto.
- **Letras sincronizadas:** o Rust consulta o LRCLIB (`https://lrclib.net/api/get` por título, artista, álbum e duração; `syncedLyrics` na resposta) com um User-Agent que nomeia o app, uma requisição por faixa, com cache no perfil. Ele envia as linhas com tempo a um módulo de injeção (`app/injections/lyrics/`) como dado JSON. O módulo as mostra na aba Letra do site com a linha atual destacada, seguindo o tempo do `<video>`, e não mexe na letra do próprio site quando o LRCLIB não tem nenhuma. A página não busca nada novo e não chama nada no Rust para isso.
- **Modo ambiente:** uma cópia desfocada e brilhante da capa atrás do player (módulo de injeção, CSS/canvas). Custa GPU e um pouco de CPU com a janela aberta, e pausa enquanto escondido. Desligado por padrão.
- **Pronto quando:** cada plugin funciona e liga/desliga pela janela de configurações, cada um tem sua linha medida (ligado e desligado) na tabela, e todos ligados, fora o modo ambiente, ficam dentro das metas (≤ 480 MB privados, ≤ 750 MB WS, ≤ 4% de CPU). O custo do modo ambiente vai na descrição dele.

### Depois (fora do escopo atual)

| Candidato | Tipo |
|---|---|
| Mais visuais (fontes, modo compacto, capa grande) | CSS |
| Botões de mídia na miniatura da barra de tarefas | Rust (Win32) |
| Equalizador e compressor de áudio (Web Audio no `<video>`) | JS; antes, decidir se quebra a regra de não mexer no que o player toca |

## Reproduzir as medições

O `measure.ps1` mede qualquer exe (padrão: o build release do app). Feche o app antes: duas instâncias no mesmo perfil dividem o processo browser do WebView2 e a árvore sai errada. O `--measure` faz o app imprimir o tempo até a barra do player (`player_bar_ms`) e, a cada 10 s, o tempo do `<video>` e se está logado.

```powershell
cd app; cargo build --release; cd ..
$a = '--measure', '<url da faixa>'
./measure.ps1 -Name playing -ArgumentList $a
./measure.ps1 -Name tray -ArgumentList $a -HideAfter 25 -Wait 50
./measure.ps1 -Name tray-1h -ArgumentList $a -HideAfter 25 -Minutes 60
./measure.ps1 -Name mini -ArgumentList $a -MiniAfter 25 -Wait 50                       # mini player aberto (Fase 5)
1..3 | % { ./measure.ps1 -Name final-$_ -Exe app/target/dist/feathertune.exe -ArgumentList $a }   # tabela da Fase 4
./median.ps1 -Prefix final-                                                                       # mediana das 3 rodadas
./measure.ps1 -Name spike -Exe spike/target/release/spike.exe -ArgumentList '"<url>"', 'spike/data/x'
```

`median.ps1 -Prefix <p>` imprime a mediana e a faixa min–max de cada `results/<p>*.json`, para as linhas de 3 rodadas das tabelas.

`-HideAfter` manda WM_CLOSE para a janela (o app vai para a bandeja); `-MiniAfter` roda `feathertune.exe --mini-player`, que abre o mini player na instância que está rodando. O app usa o perfil real, então mede logado.

## Referências

- WebView2: [modelo de processos](https://learn.microsoft.com/microsoft-edge/webview2/concepts/process-model), [performance](https://learn.microsoft.com/microsoft-edge/webview2/concepts/performance), [MemoryUsageTargetLevel](https://learn.microsoft.com/dotnet/api/microsoft.web.webview2.core.corewebview2.memoryusagetargetlevel)
- Tauri 2: [WebviewWindowBuilder](https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindowBuilder.html) (`initialization_script`, `additional_browser_args`)
- Login embutido no WebView2: [WebView2Feedback #1584](https://github.com/MicrosoftEdge/WebView2Feedback/issues/1584)
