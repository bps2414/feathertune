# Contribuindo com o feathertune

*English: [CONTRIBUTING.md](CONTRIBUTING.md)*

Obrigado por querer ajudar. Este guia explica como mandar um pull request (PR) que eu consiga revisar e aceitar rápido. Ele é longo de propósito: a maior parte dos PRs que voltam para ajuste voltam por algo que está aqui.

Se você usa um agente de código (Claude Code, Copilot, Codex...), as instruções dele estão em [AGENTS.md](AGENTS.md). As regras são as mesmas deste guia.

## Antes de começar

- **É um projeto pessoal, sem suporte formal.** Eu leio tudo, mas não há prazo de resposta. Um PR pode ficar dias sem revisão.
- **Mudança pequena e óbvia** (typo, bug claro, texto de interface): pode abrir o PR direto.
- **Mudança grande ou que muda o comportamento** (plugin novo, tela nova, dependência nova, mexer em áudio, letras, privacidade ou no que a página pode chamar): abra uma **issue antes** e espere eu responder. Assim você não gasta uma semana numa coisa que eu não vou aceitar.
- **Procure antes.** Veja as issues e PRs abertos e a seção "Ainda não conferido" e "Ideias" do [docs/DESIGN.pt-BR.md](docs/DESIGN.pt-BR.md). Pode já existir decisão tomada ou uma coisa descartada com o motivo.
- **Bug?** Anexe o arquivo de **Exportar diagnóstico** (Configurações > Advanced). Ele cita as faixas tocadas desde que o app abriu; leia antes de enviar e apague o que não quiser publicar.

## O que eu aceito e o que eu não aceito

Estes limites vêm do [AGENTS.md](AGENTS.md) e não mudam, nem com um bom motivo. Um PR que cruze qualquer um deles é fechado.

- **Anúncios:** nunca bloquear, esconder, pular, encurtar, silenciar, esmaecer nem cobrir. Todo efeito de áudio fica em bypass enquanto um anúncio toca. O filtro de rede é só de listas de privacidade (analytics de terceiros).
- **Áudio:** só processar e medir localmente, no som da própria página, para quem está ouvindo. Nunca gravar, guardar, enviar nem recodificar áudio, e nunca buscar um stream. Só números derivados (tempo, tom, contorno de volume) podem ser guardados no perfil.
- **Sem API privada, extração de stream, download nem DRM.**
- **Sem spoof de user-agent e sem flags de automação.**
- **Credenciais:** o app nunca digita nem guarda login de conta. Quem confere login e conta sou eu.
- **A página é não confiável.** Ela só pode chamar `player_state`, `audio_facts` e `mix_open`, com payloads validados e sem disparar ação no player. Não amplie `capabilities/` nem `allowed()` sem escrever o motivo em `docs/DESIGN.md`.
- **Nomes de marca** ficam fora do nome do repositório, da interface, do README e dos docs. Domínios e seletores CSS no código são aceitos quando funcionais.
- **Dados pessoais** não entram em commit: nada de caminhos locais, nome de usuário, e-mail, token, chave de API nem histórico de escuta.

Outras coisas que eu costumo recusar: dependência nova sem necessidade clara (cada crate aumenta o tempo de build), refatoração grande misturada com correção, e recurso que pesa no uso de CPU ou memória sem ter como desligar.

## Preparando o ambiente

O app só roda no **Windows 10/11** (usa WebView2 e a crate `windows`). Não compila nem roda em Linux ou macOS.

1. Instale o runtime WebView2 (já vem no Windows 11) e o Rust **stable** com o alvo MSVC (Build Tools do Visual Studio com "Desenvolvimento para desktop com C++").
2. Faça um **fork** no GitHub e clone o seu fork.
3. Crie uma branch a partir da `main`, com um nome que diga o assunto: `fix/letra-some-apos-anuncio`, `feat/...`, `docs/...`.
4. Os comandos do Cargo rodam dentro de `app/`:

```powershell
cd app
cargo check
```

`.cargo/config.toml` já liga com `rust-lld`; não mude isso. Para rodar o app: `cargo build --release` e depois `.\target\release\feathertune.exe`.

O app lê `%LOCALAPPDATA%\feathertune\config.toml`. Um build local não tem as chaves do Last.fm e do Discord dos releases; se precisar delas, ponha as suas no `config.toml` (veja o README). Você não precisa delas para a maior parte do trabalho.

## Disciplina de build (importante)

Os builds são lentos e eu quero que eles não se repitam à toa. Tempos depois de mudar um arquivo: `cargo check` ~1 s, `cargo build --release` ~9 s, `cargo clippy` ~4 s. Um build do zero leva 1 a 2 minutos por perfil, e `--profile dist` leva vários minutos.

- Enquanto edita, use **só `cargo check`**.
- Antes de abrir o PR, rode **uma vez**: `cargo fmt`, `cargo clippy --all-targets --locked -- -D warnings` e `cargo test --locked`. É o mesmo que o CI roda; se passar aí, o CI passa.
- Só compile (`cargo build --release`) quando for **rodar** o exe, e uma vez por lote de mudanças.
- Use o perfil `release` para testar a mão. `--profile dist` é só para instalador e medição de release.
- **Nunca** `cargo clean`, apague `target/`, desligue cache ou build incremental, nem defina `CARGO_INCREMENTAL=0` localmente.
- Não rode dois builds ao mesmo tempo: eles brigam pelo mesmo lock de `target/`.

## Regras de código

- **Tudo em inglês:** identificadores, comentários, nomes de arquivo, chaves do `config.toml`, logs e mensagens de commit.
- **Estilo:** siga o código ao redor. `rustfmt` com largura 120 (`app/rustfmt.toml`); `cargo fmt` resolve.
- **Mudança mínima.** Sem abstração especulativa, sem arquivo novo que ninguém pediu, sem "já que estou aqui" em código fora do assunto do PR.
- **Dependências:** não adicione crate nem feature do Cargo sem necessidade; prefira o que já está no `Cargo.toml`. Se for realmente necessário, explique no PR o que ganha e quanto custa de build.
- **Flags do Chromium:** toda flag de feature vai no único `--disable-features` de `BROWSER_ARGS` (o Chromium só respeita o último).
- **Textos da interface:** escreva em inglês dentro de `t("...")` (ou `tf` para `{}`) e adicione a tradução em `app/locales/pt-BR.json`. Sem entrada, o app mostra o inglês, então não quebra, mas eu peço a tradução. A janela de configurações é traduzida pelo mesmo arquivo.
- **Interface e performance** (nativa ou HTML):
  - desenhe só quando algo mudar e invalide só a região que muda;
  - mantenha recursos de GPU (brushes, layouts, bitmaps) entre os quadros;
  - toda animação tem interruptor no plugin, roda em **um** timer só enquanto houver algo na tela para mover, para quando a janela está escondida, minimizada ou em segundo plano, e continua de onde parou;
  - os efeitos de animação do Windows e `prefers-reduced-motion` não desligam a animação; quem desliga é o interruptor do plugin.
- **Plugins novos:** vêm **ligados por padrão**, exceto os da aba Experimental (desligados até ficarem confiáveis). Registre o custo em `plugins.rs` e em `docs/DESIGN.md`, ou escreva "not measured yet" se você não mediu. A medição de verdade (`measure.ps1`) sou eu que rodo.
- **Testes:** os testes ficam dentro dos módulos (`#[cfg(test)]`). Se você corrige um bug em lógica pura (letras, mixes, privacidade, configurações), um teste que reproduz o bug é muito bem-vindo.
- **Documentação em par:** os docs existem em inglês e em pt-BR. Se você muda um, muda o par (`README.md` e `README.pt-BR.md`, `docs/DESIGN.md` e `docs/DESIGN.pt-BR.md`, `CONTRIBUTING.md` e `CONTRIBUTING.pt-BR.md`). Se você não escreve em um dos idiomas, diga no PR e eu faço a outra versão.
- **Versão e CHANGELOG não são seus:** não mude `version` em `Cargo.toml` e `tauri.conf.json`, nem `CHANGELOG.md` e `CHANGELOG.pt-BR.md`, nem crie tags. Release é feito por mim. Descreva o que mudou para o usuário no PR que eu escrevo a entrada.

## O que testar antes de abrir

O CI só confere formatação, clippy e testes unitários, num runner Windows. Ele **não** abre o app, não toca música e não tem conta. Por isso:

- Rode o app (`cargo build --release`) e use o que você mudou de verdade, no caminho principal e em um caso de borda (janela escondida, anúncio tocando, troca de faixa, primeira execução sem `config.toml`).
- No PR, diga **o que você testou e como** e **o que não testou**. Se algo exige conta logada, medição (`measure.ps1`) ou uma situação que você não consegue reproduzir, escreva isso em vez de dizer que funciona. Eu confiro essa parte.
- Não coloque no PR (texto, print, gif ou log) nada pessoal: nome de usuário, caminhos locais, tokens, e-mail, nome de conta ou o que você ouve.
- Se mexer na interface, anexe um print ou um gif curto (sem dados pessoais).

## Abrindo o PR

1. Atualize sua branch com a `main` (`git fetch` e rebase) antes de abrir, para evitar conflito.
2. **Um assunto por PR.** Dois assuntos viram dois PRs; PR pequeno é revisado mais rápido.
3. **Commits e título em inglês**, no imperativo, começando pela área, como já está no histórico: `Lyrics: keep the line lit until its end`, `Docs: fix the equalizer table`. Commits que só consertam o commit anterior podem ser juntados (squash) antes de abrir.
4. Abra o PR contra a `main` do repositório original. O formulário já vem com o modelo (`.github/PULL_REQUEST_TEMPLATE.md`); preencha tudo. Pode escrever em inglês ou em português.
5. Se ainda não está pronto, abra como **rascunho (draft)**. Só tire do rascunho quando o CI estiver verde e o modelo preenchido.
6. Link para a issue relacionada (`Closes #123`) quando houver.

Se for o seu primeiro PR aqui, o GitHub pode pedir que eu aprove a execução do CI antes de ele rodar.

## O que acontece depois

- **CI:** se ficar vermelho, leia o log (`fmt`, `clippy` ou `test`) e corrija na mesma branch com um novo push. Não precisa fechar e abrir outro PR.
- **Revisão:** eu comento no código. Pode haver pedido de mudança, pergunta ou uma recusa com o motivo. Responda no mesmo lugar. Não leve a mal: recusar faz parte de manter o app pequeno e leve.
- **Conflitos:** se a `main` andou, faça rebase na sua branch e dê push.
- **Merge:** eu faço o merge, em geral como **squash**, com uma mensagem minha em inglês. O seu nome continua como autor da contribuição.
- **PR parado:** se eu pedi mudança e não houve resposta por cerca de 30 dias, posso fechar com um aviso. Você pode reabrir quando quiser continuar.
- **Release:** a sua mudança chega aos usuários no próximo release, com uma entrada no CHANGELOG escrita por mim.

## Usando agentes de código

Pode usar. O PR continua sendo seu: leia o diff inteiro, rode e teste o que o agente fez, e diga no PR que usou um agente. O agente tem que seguir o [AGENTS.md](AGENTS.md) (as mesmas regras de build, de código e os limites acima). PR gerado e enviado sem revisão humana é fechado.

## Segurança

Falha de segurança não vai em issue pública. Veja o [SECURITY.md](SECURITY.md).

## Licença das contribuições

O projeto usa a licença [MIT](LICENSE). Ao enviar um PR, você declara que escreveu o código (ou que tem o direito de enviá-lo) e concorda que ele seja distribuído sob a MIT. Não copie código de projeto com licença incompatível nem de fonte desconhecida.

## Convivência

Seja respeitoso e objetivo, com quem revisa e com quem pergunta. Crítica é sobre o código, não sobre a pessoa. Comentários ofensivos, assédio ou spam são apagados e a conta pode ser bloqueada.

## Checklist rápido

- [ ] Abri uma issue antes, se a mudança é grande
- [ ] Não cruzo nenhum limite da seção "O que eu aceito e o que eu não aceito"
- [ ] `cargo fmt`, `cargo clippy --all-targets --locked -- -D warnings` e `cargo test --locked` passam em `app/`
- [ ] Rodei o app e testei o que mudei; escrevi no PR o que não consegui testar
- [ ] Texto novo de interface está em `t("...")` e em `locales/pt-BR.json`
- [ ] Docs atualizados nos dois idiomas, quando mexi em algum
- [ ] Não mexi em versão, CHANGELOG nem tags
- [ ] Nada pessoal no código, nos commits, nos prints ou nos logs
- [ ] Um assunto só, commits em inglês, branch atualizada com a `main`
