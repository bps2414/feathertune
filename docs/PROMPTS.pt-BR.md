# Prompts das fases

*English: [PROMPTS.md](PROMPTS.md).*

Uma sessão nova por fase, em ordem, com o modelo indicado. Copie o bloco inteiro. As Fases 1 a 4 estão commitadas (as Fases 3 e 4 têm conferências do mantenedor em aberto). As regras do repo (disciplina de build, limites legais, idioma) estão no [AGENTS.md](../AGENTS.md), que toda sessão lê.

## Fase 3: Integrações (Sonnet 5.5, esforço high)

```
Execute a Fase 3 do feathertune, descrita em docs/PLAN.pt-BR.md ("Fase 3: Integrações").

Antes, leia o AGENTS.md, o docs/PLAN.pt-BR.md e o código em app/. Leia com atenção o módulo `Player` (a API está documentada no topo de app/src/player.rs): é a única parte da Fase 2 que você consome. Rode `git log --oneline` para confirmar que as Fases 1 e 2 estão commitadas; se não estiverem, pare e me avise.

Escopo: só a Fase 3: plugins single-instance, autostart (desligado por padrão, opção no menu da bandeja) e window-state; notificação de troca de faixa só com a janela escondida; atalhos globais configuráveis no config.toml do perfil; Discord Rich Presence; Last.fm (now playing + scrobble pela regra de 30 s / metade / 4 min, com fila offline). Não mexa nos scripts injetados, no comando `player_state`, no WebView, nas flags nem no SMTC.

Regras:
- Use as versões atuais dos plugins oficiais do Tauri 2 e confira a documentação de cada crate antes de usar.
- A chave de API do Last.fm é minha: leia de configuração local fora do Git, nunca do código. A session key vai para o Windows Credential Manager (crate keyring).
- A autenticação do Last.fm e as confirmações do Discord e do scrobble são feitas por mim com as minhas contas. Nunca digite credenciais.
- Se estiver numa sessão na nuvem (Linux), você não consegue rodar o app: use o CI do Windows (veja o AGENTS.md) e deixe para mim os testes na bandeja e a medição.

Pronto quando: cada item funciona com o app na bandeja, eu confirmei o Discord e o scrobble no meu perfil do Last.fm, a CPU tocando continua ≤ 4% (measure.ps1) e os dois arquivos do plano têm "Fase 3: concluída em <data>" com a medição.

No final, um commit com a fase inteira: "Phase 3: integrations".
```

## Fase 4: Dieta e entrega (Sonnet 5.5, esforço high)

```
Execute a Fase 4 do feathertune, descrita em docs/PLAN.pt-BR.md ("Fase 4: Dieta e entrega").

Antes, leia o AGENTS.md, o docs/PLAN.pt-BR.md, o código em app/ e o measure.ps1. Rode `git log --oneline` para confirmar que as Fases 1 a 3 estão commitadas; se não estiverem, pare e me avise.

Escopo: só a Fase 4.
1. Linha de base do app atual (3 rodadas, mediana): tocando com a janela aberta e na bandeja.
2. Experimentos um por vez, cada um medido igual: CSS que remove animações/transições/backdrop-filter, como módulo de injeção `performance`, e flags de features não usadas. Só fica o que mostrar ganho; cada experimento (mantido ou descartado) entra numa tabela nos dois arquivos do plano.
3. Filtro de rastreadores: WebResourceRequested do WebView2 via with_webview + crate adblock só com lista de privacidade. Nunca lista de anúncio, nunca nada que mude o que o player toca. Se travar nesse item, pare só ele, documente onde travou no plano e siga.
4. Instalador NSIS do Tauri com o bootstrapper do WebView2, gerado com --profile dist. Ele cria o atalho no Menu Iniciar com o AUMID do app e o desinstalador remove.
Fora do escopo: os plugins (Fases 5 e 6).

Regras:
- As medições rodam no Windows com o measure.ps1. Numa sessão na nuvem, implemente e deixe o CI compilar; eu rodo as medições.

Pronto quando: a tabela final cliente Electron × feathertune está nos dois arquivos do plano (3 rodadas, mesmo critério), o instalador instala e desinstala limpo (o perfil só sai se o usuário pedir) e os dois arquivos do plano têm "Fase 4: concluída em <data>".

No final, um commit com a fase inteira: "Phase 4: slimming and installer".
```

## Fase 5: Casca nativa (Opus 5.5, esforço medium)

```
Execute a Fase 5 do feathertune, descrita em docs/PLAN.pt-BR.md ("Fase 5: Casca nativa").

Leia primeiro o AGENTS.md, o docs/PLAN.pt-BR.md e o código em app/ (main.rs, player.rs, smtc.rs, privacy.rs, settings.rs, lastfm.rs, capabilities/). Rode `git log --oneline` para confirmar que as Fases 1 a 4 estão commitadas; se não estiverem, pare e me avise.

Escopo: só a Fase 5.
1. Barra de título nativa: decorations desligado, uma faixa desenhada pelo Rust com Direct2D/DirectWrite (nome, menu, minimizar, maximizar/restaurar, fechar), o site numa webview filha abaixo dela (multiwebview do Tauri, feature `unstable`), WM_NCHITTEST nativo com snap layouts e bordas de redimensionar, ciente de DPI, claro/escuro e um gancho de cor de tema para a Fase 6.
2. Janela de configurações: HTML/CSS nosso servido por um esquema de URI próprio (nunca frontendDist), capability própria só para essa janela e esse esquema, criada ao abrir e destruída ao fechar; grava o config.toml mantendo os comentários; lista todo plugin com seu interruptor.
3. Modelo de plugins: todo recurso opcional (módulo de injeção e/ou integração Rust) declarado com id, nome, descrição, custo e interruptor no config.toml, listado pela janela de configurações. Sem DLL, sem código baixado.
4. API key e secret do Last.fm vindos dos secrets do CI FEATHERTUNE_LASTFM_API_KEY / FEATHERTUNE_LASTFM_API_SECRET via option_env! no job dist; o config.toml tem prioridade; builds sem eles funcionam como hoje.
5. Mini player nativo sempre no topo (Direct2D, capa decodificada com WIC, baixada pelo Rust só dos hosts de imagem do site), alimentado só pelo Player; abrir esconde a janela principal com memória Low.
Fora do escopo: cor do álbum, letras e modo ambiente (Fase 6).

Regras:
- A capability do site não muda: só player_state, só a webview do site. A capability da janela de configurações não pode alcançar a webview do site, e o site não pode alcançar os comandos da janela de configurações. Escreva no plano como isso é garantido.
- Confira a documentação atual do multiwebview, do esquema de URI próprio e das capabilities do Tauri 2 antes de usar (ctx7), e as features do crate `windows` que adicionar. Nenhum crate novo, fora o toml_edit se o `toml` não mantiver comentários.
- Mantenha tudo o que já funciona na webview filha: memória Low ao esconder, o filtro de rastreadores, SMTC/flyout, single-instance, window-state, `--measure`.
- Se o multiwebview travar (foco, teclado, memória Low ou SMTC quebrando na webview filha), pare o item 1, use o plano reserva da moldura do sistema colorida pelo DWM descrito no plano, escreva o motivo no plano e siga.
- Eu crio os dois secrets no GitHub e confirmo o login do Last.fm com a minha conta. Nunca digite credenciais.
- Numa sessão na nuvem (Linux) você não roda o app: use o CI do Windows e deixe as conferências da janela e as medições para mim.

Pronto quando: a lista "Pronto quando" da Fase 5 no plano for cumprida, medida com o measure.ps1 (3 rodadas, mediana) e anotada na tabela dos dois arquivos do plano, a situação do README atualizada nos dois idiomas, e os dois arquivos do plano disserem "Fase 5: concluída em <data>".

No fim, um commit com a fase inteira: "Phase 5: native shell".
```

## Fase 6: Visual (Sonnet 5.5, esforço high)

```
Execute a Fase 6 do feathertune, descrita em docs/PLAN.pt-BR.md ("Fase 6: Visual").

Leia primeiro o AGENTS.md, o docs/PLAN.pt-BR.md e o código em app/, principalmente o modelo de plugins da Fase 5, a janela de configurações, o gancho de cor de tema da barra de título e o decodificador WIC da capa. Rode `git log --oneline` para confirmar que a Fase 5 está commitada; se não estiver, pare e me avise.

Escopo: só a Fase 6, cada item um plugin declarado no modelo da Fase 5 e ligável pela janela de configurações.
1. Cor do álbum: cor dominante da capa atual (decodificador da Fase 5, uma vez por faixa) aplicada na barra de título, no mini player e na página por uma variável CSS em app/injections/theme/. Só apresentação, contraste legível do texto.
2. Letras sincronizadas: o Rust busca https://lrclib.net/api/get (track_name, artist_name, album_name, duration; syncedLyrics) com um User-Agent que nomeia o app, uma requisição por faixa, com cache no perfil, e envia as linhas com tempo como dado JSON para app/injections/lyrics/, que as mostra na aba Letra do site seguindo o tempo do <video>. Quando o LRCLIB não tiver nada, não mexa na letra do próprio site.
3. Modo ambiente: capa desfocada e brilhante atrás do player (módulo de injeção), pausado enquanto escondido, desligado por padrão.

Regras:
- A página continua não confiável: não busca nada novo nem chama nada no Rust para isso; o dado vai só Rust → página, serializado como JSON, nunca como código montado com strings. Não amplie capabilities/.
- Os módulos de injeção continuam só de apresentação e nunca mexem em anúncios nem no que o player toca.
- Meça cada plugin ligado e desligado com o measure.ps1 (3 rodadas, mediana) e aplique a regra da Fase 4; os padrões seguem os números, fora o modo ambiente, que fica desligado.
- Numa sessão na nuvem (Linux) você não roda o app: use o CI do Windows e deixe as conferências e medições para mim.

Pronto quando: o "Pronto quando" da Fase 6 no plano for cumprido, todas as medições estiverem na tabela dos dois arquivos do plano, a situação do README atualizada nos dois idiomas, e os dois arquivos do plano disserem "Fase 6: concluída em <data>".

No fim, um commit com a fase inteira: "Phase 6: look".
```
