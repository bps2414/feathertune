# Security policy

*Português: veja a seção no fim.*

feathertune hosts a web page it does not control, so the boundary between that page and the app matters. The model is in [docs/DESIGN.md](docs/DESIGN.md) (the page may only call `player_state`, `audio_facts`, `mix_open` and `ext_call`, with validated payloads).

## Reporting a vulnerability

Please do not open a public issue for a security problem. Use GitHub's private reporting instead: the **Security** tab of the repository, then **Report a vulnerability**.

Useful things to include: the version (the installer's file name or the release you installed), what you did, what you expected and what happened. Anything that lets the page, a lyrics source or another site run an action in the app, read local files or reach the credentials is in scope, and so is anything that lets someone other than a paired phone use the phone remote's port. What an imported extension does with the permissions it was granted is not: extensions are the user's own code.

This is a personal project with no formal support: I read reports as soon as I can, but there is no guaranteed response time.

Supported version: the latest release (the app updates itself).

## Português

Não abra uma issue pública para falha de segurança. Use o relato privado do GitHub: aba **Security** do repositório, depois **Report a vulnerability**. Informe a versão, o que você fez, o que esperava e o que aconteceu. Está no escopo tudo que deixa a página, uma fonte de letras ou outro site disparar uma ação no app, ler arquivos locais ou chegar às credenciais, e tudo que deixa alguém além de um celular pareado usar a porta do controle remoto; o que uma extensão importada faz com as permissões que recebeu não está (extensões são código do próprio usuário). É um projeto pessoal sem suporte formal: leio os relatos assim que puder, sem prazo garantido. A versão suportada é a última (o app se atualiza sozinho).
