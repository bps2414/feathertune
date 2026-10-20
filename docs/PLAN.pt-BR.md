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
| 3 | pendente | Tocando, janela aberta, todas as integrações ligadas (3 rodadas, mediana) | | | | | |
| 3 | pendente | Tocando, na bandeja, todas as integrações ligadas (3 rodadas, mediana) | | | | | |

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
├─ teclas de mídia / flyout do Windows (SMTC)            CSS injetado: corta animação e blur (Fase 4)
├─ Discord Rich Presence, Last.fm, notificação de faixa
└─ filtro de rastreadores (só listas de privacidade, Fase 4)
```

- **Rust:** tudo do Windows (bandeja, SMTC, atalhos, autostart, notificações), as integrações, o controle de memória (escondeu → `MemoryUsageTargetLevel=Low`, mostrou → `Normal`) e o filtro de rede.
- **Página:** o site como é entregue. Login, biblioteca, letra, rádio e recomendações já vêm prontos; o app não reimplementa nada disso.
- **Ponte:** um script injetado (`initialization_script`) observa o player e manda ao Rust um JSON com título, artista, capa, tempo e estado. O Rust trata como dado não confiável: valida e não executa nada. Os comandos Rust → página (pausar, próxima) clicam nos botões do player; se o site mudar a interface, quebram uns 3 seletores CSS, não um protocolo.
- **Segurança:** a página remota não recebe nenhum comando do Tauri além do relato de estado; a navegação fica presa aos domínios do site e do login (o resto abre no navegador padrão).

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

### Fase 1: Casca (concluída em 19/10/2026)

App Tauri 2.12 em `app/`, sem frontend próprio: a janela principal abre o site. Perfil do WebView2 em `%LOCALAPPDATA%\feathertune`. Flags do Chromium num único `--disable-features` (o Chromium só respeita o último). Navegação presa aos domínios do site e do login. Fechar esconde na bandeja com `MemoryUsageTargetLevel=Low`. Bandeja: mostrar/esconder, tocar/pausar, próxima, sair.

**Injeções como módulos** (semente dos plugins futuros): todo JS/CSS que entra na página fica em `app/injections/<id>/` (`page.js` e/ou `style.css`), tem liga/desliga em `[injections]` no `config.toml` do perfil e é montado num único `initialization_script`, cada módulo isolado num `try/catch` próprio.

Resultado: tocando 408 MB privados e 2,8% de CPU, bandeja 167 MB de WS, abertura até a barra do player 2,0 s, o login persiste ao reabrir. **Pendente:** o teste de 1 h na bandeja (`./measure.ps1 -Name tray-1h -ArgumentList $a -HideAfter 25 -Minutes 60`, sem abrir o app durante a hora; passa se o WS final ≤ WS aos 5 min + 10%).

### Fase 2: Ponte e teclas de mídia (concluída em 20/10/2026)

- Injeção `core` (sempre ligada, só no frame principal) → comando único `player_state` → `Player` (`app/src/player.rs`, API documentada no topo). Capability `app/capabilities/player-bridge.json`: só esse comando, só a janela `main`, só a origem do site e nada para origem local. O `frontendDist` saiu do `tauri.conf.json`: com ele, o Tauri tratava o site remoto como origem *local*, e qualquer capability local futura vazaria para a página.
- O SMTC do próprio WebView2 aparecia como `msedgewebview2.exe`, então foi desligado (`HardwareMediaKeyHandling`) e o app publica o seu (`app/src/smtc.rs`, preso à janela principal).
- **Nome no flyout:** um exe solto aparece como "Unknown app". O que resolve é o processo ter um AppUserModelID (`dev.feathertune.desktop`) e existir um atalho no Menu Iniciar com o mesmo ID. O app cria `%APPDATA%\Microsoft\Windows\Start Menu\Programs\feathertune.lnk` se não existir; na Fase 4 o instalador deve criá-lo com o mesmo AUMID e o desinstalador deve removê-lo.
- Teclas de mídia com o app na bandeja: tocar/pausar ≤ 16 ms, próxima ~0,5 s, anterior ~0,2 s até o novo estado chegar no Rust. CPU tocando: mediana de 3,6%, 408 MB privados.

### Fase 3: Integrações (Sonnet 5.5 high) — código escrito em 20/10/2026, conferências do mantenedor e medição pendentes

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
- **Limites da sessão na nuvem:** `cargo clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings` e `cargo fmt --check` passam no Linux; o app não foi executado. Pendente para o mantenedor, no Windows: cada item na bandeja, a conferência do Discord, um scrobble de verdade e o `measure.ps1` (3 rodadas, mediana) das duas linhas acima. Quando tudo passar, troque a nota do título por "Fase 3: concluída em <data>" e preencha a tabela.

### Fase 4: Dieta e entrega (Sonnet 5.5 high)

- Experimentos um por vez, cada um medido com `measure.ps1` (3 rodadas, mediana). Só fica o que mostrar ganho:
  - CSS que remove animações, transições e `backdrop-filter`, como módulo de injeção `performance`;
  - flags de features que o app não usa.
- Filtro de rastreadores: `WebResourceRequested` do WebView2 (via `with_webview`) + crate `adblock` com **só lista de privacidade**, nunca lista de anúncio, e nada que mude o que o player toca. É o único item de interop COM da fase: se travar, devolva só ele ao Opus.
- Instalador NSIS do Tauri com o bootstrapper do WebView2, gerado com `--profile dist`.
- **Pronto quando:** a tabela final cliente Electron × feathertune está aqui (3 rodadas, mesmo critério), o instalador instala e desinstala limpo, e a desinstalação só remove o perfil se o usuário pedir.

## Depois da Fase 4: plugins (futuro, fora do escopo atual)

Objetivo: plugins que você liga e desliga. Um módulo de injeção (`app/injections/<id>/`), uma parte Rust compilada no app que implementa um trait `Plugin` (recebe o estado, adiciona itens ao menu), ou os dois. Liga/desliga num menu "Plugins" da bandeja, salvo no `config.toml`. Sem carregar DLL nem código baixado em tempo de execução. Cada plugin é medido ligado e desligado e o custo vai na descrição dele; todos ligados não podem passar das metas de CPU e RAM.

| Candidato | Tipo |
|---|---|
| Temas e visual (cores, fonte, modo compacto, capa grande) | CSS |
| Performance (o módulo da Fase 4, com níveis) | CSS |
| Equalizador e compressor de áudio (Web Audio no `<video>`) | JS |
| Botões de mídia na miniatura da barra de tarefas | Rust (Win32) |

Antes do primeiro plugin, uma sessão Opus 5.5 medium formaliza o trait `Plugin` e o menu.

## Reproduzir as medições

O `measure.ps1` mede qualquer exe (padrão: o build release do app). Feche o app antes: duas instâncias no mesmo perfil dividem o processo browser do WebView2 e a árvore sai errada. O `--measure` faz o app imprimir o tempo até a barra do player (`player_bar_ms`) e, a cada 10 s, o tempo do `<video>` e se está logado.

```powershell
cd app; cargo build --release; cd ..
$a = '--measure', '<url da faixa>'
./measure.ps1 -Name playing -ArgumentList $a
./measure.ps1 -Name tray -ArgumentList $a -HideAfter 25 -Wait 50
./measure.ps1 -Name tray-1h -ArgumentList $a -HideAfter 25 -Minutes 60
./measure.ps1 -Name final -Exe app/target/dist/feathertune.exe -ArgumentList $a   # tabela da Fase 4
./measure.ps1 -Name spike -Exe spike/target/release/spike.exe -ArgumentList '"<url>"', 'spike/data/x'
```

`-HideAfter` manda WM_CLOSE para a janela (o app vai para a bandeja). O app usa o perfil real, então mede logado.

## Referências

- WebView2: [modelo de processos](https://learn.microsoft.com/microsoft-edge/webview2/concepts/process-model), [performance](https://learn.microsoft.com/microsoft-edge/webview2/concepts/performance), [MemoryUsageTargetLevel](https://learn.microsoft.com/dotnet/api/microsoft.web.webview2.core.corewebview2.memoryusagetargetlevel)
- Tauri 2: [WebviewWindowBuilder](https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindowBuilder.html) (`initialization_script`, `additional_browser_args`)
- Login embutido no WebView2: [WebView2Feedback #1584](https://github.com/MicrosoftEdge/WebView2Feedback/issues/1584)
