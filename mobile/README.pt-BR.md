# feathertune remote (app do celular)

English: [README.md](README.md)

O app de celular que controla o feathertune num PC pareado pela rede local. Plano, modelo de segurança e protocolo: [docs/REMOTE.pt-BR.md](../docs/REMOTE.pt-BR.md).

```
pnpm install
pnpm typecheck && pnpm lint && pnpm test
```

## `pnpm fake-pc`: um PC sem o PC

`pnpm fake-pc` faz o papel do PC no protocolo, no seu terminal, para construir e testar o app sem uma máquina Windows e sem conta. Usa o mesmo código de `src/noise` do app e segue à risca a seção Protocol do plano, recusas incluídas.

- Escuta na sua rede local (porta 47652, `--port` para mudar) e mostra o QR de pareamento e o texto `feathertune://pair?…`. Escaneie ou cole em até 2 minutos.
- Quando um celular pareia, o terminal mostra o nome dele e o código de 6 dígitos (o celular mostra o mesmo) e pergunta `y` ou `n`.
- Depois disso o celular recebe uma faixa e uma fila inventadas que andam sozinhas, e os comandos dele (play, pause, próxima, anterior, seek, volume, tocar uma linha da fila) funcionam.
- Teclas: `a` liga ou desliga o modo anúncio (tudo menos play e pause é recusado com `ad`), `p` abre um novo QR de pareamento, `d` lista os celulares pareados, `q` sai.
- Outras opções: `--name <texto>` (o nome do PC), `--artwork <url https>` (capa de todas as faixas; nenhuma por padrão), `--no-rate-limit` (o PC de verdade aceita 5 conexões por endereço por minuto).
- Não toca áudio e descarta tudo de fora da rede local. Na primeira execução o Firewall do Windows pode perguntar sobre o Node: permita redes privadas.
- A chave do PC falso e os celulares pareados ficam em `tools/fake-pc/fake-pc.json` (fora do git); apague-o para recomeçar.

## A camada de conexão e o que precisa de celular

`src/remote/` é o lado do celular: `protocol.ts` (QR, código, mensagens, códigos de fechamento), `client.ts` (um socket e o seu handshake) e `remote.ts` (os PCs pareados, a reconexão, os pings, o segundo plano), com armazenamento, sockets e rede injetados. `tools/fake-pc/remote.test.ts` roda tudo contra o PC falso no mesmo processo (pareamento, reconexão, celular removido, PC de outra versão), então `pnpm test` cobre isso em qualquer sistema.

Só um build de desenvolvimento num celular de verdade confere o resto: a câmera e a leitura do QR, a permissão de rede local (o pedido do Android 17, o do próprio iOS na primeira conexão), o armazenamento seguro e uma conexão `ws://` ao PC pelo Wi-Fi.
