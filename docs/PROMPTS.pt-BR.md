# Prompts das fases

*English: [PROMPTS.md](PROMPTS.md).*

Uma sessão nova por fase, em ordem, com o modelo indicado. Copie o bloco inteiro. As Fases 1 e 2 estão concluídas. As regras do repo (disciplina de build, limites legais, idioma) estão no [AGENTS.md](../AGENTS.md), que toda sessão lê.

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
Fora do escopo: os plugins de "Depois da Fase 4".

Regras:
- As medições rodam no Windows com o measure.ps1. Numa sessão na nuvem, implemente e deixe o CI compilar; eu rodo as medições.

Pronto quando: a tabela final cliente Electron × feathertune está nos dois arquivos do plano (3 rodadas, mesmo critério), o instalador instala e desinstala limpo (o perfil só sai se o usuário pedir) e os dois arquivos do plano têm "Fase 4: concluída em <data>".

No final, um commit com a fase inteira: "Phase 4: slimming and installer".
```
