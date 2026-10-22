# Controle remoto pelo celular: plano

English: [REMOTE.md](REMOTE.md)

Um app de celular que controla o feathertune num PC pareado pela rede local. Ele não toca música: mostra o que o PC toca (faixa, capa, posição, fila) e manda tocar/pausar, próxima, anterior, buscar posição, volume e "tocar este item da fila". Ainda não começou; este é o plano a partir do qual quem contribuir deve construir. Dúvidas e andamento vão na issue de acompanhamento.

## Objetivos e limites

- **Só celulares pareados controlam um PC, e só o PC com que parearam.** Um celular na mesma Wi-Fi que nunca foi pareado, um código de pareamento roubado depois de usado, uma mensagem repetida ou um celular que o usuário removeu não recebem nada: nem estado, nem comandos.
- Só rede local. Sem relay na nuvem, sem conta, sem redirecionamento de porta, sem UPnP. Uso fora de casa só pela VPN do próprio usuário (ex.: uma VPN mesh), que parece LAN para os dois lados.
- Desligado por padrão (aba Experimental), como todo recurso que abre algo para fora.
- As regras do [AGENTS.md](../AGENTS.md) valem sem mudança: nada durante um anúncio é pulado, encurtado ou silenciado (buscar posição, próxima, anterior e saltos na fila são recusados enquanto um toca, e o volume não muda); nenhum áudio sai do PC; nada de nomes de marca no nome do app, na loja ou na interface.

## Stack

**Celular: Expo (React Native), SDK 58, TypeScript.** Escolhido em vez das alternativas porque:

| Opção | Por que não (para este app) |
|---|---|
| Tauri 2 mobile | Compartilharia Rust com o desktop, mas a interface é um webview, o mobile é o lado menos maduro do Tauri (nem todos os plugins portados) e um build de iOS exige Xcode num Mac. O mantenedor usa só Windows. |
| Flutter | Boa interface, mas uma terceira linguagem (Dart) sem nada compartilhado, e builds de iOS também exigem Mac ou CI pago. |
| Nativo (Kotlin + Swift) | Duas bases de código para um app pequeno. |
| Interface em Rust (Dioxus, Slint) | Suporte a mobile ainda novo; toque, gestos, câmera e armazenamento seguro seriam feitos à mão. |

O Expo dá interface nativa com gestos e animações suaves, câmera (ler QR), Keychain/Keystore e vibração como módulos mantidos, e o EAS Build compila iOS na nuvem sem Mac. Bibliotecas:

- `expo-router` (telas), `react-native-reanimated` + `react-native-gesture-handler` (barra de progresso, volume, painel), `expo-image` (capa com fundo desfocado), `expo-linear-gradient` com as cores do álbum que o PC já calcula (`album_color.rs`).
- `expo-camera` (leitor de QR), `expo-secure-store` (chaves), `expo-haptics`, `expo-local-authentication` (bloqueio opcional do app).
- Criptografia: `@noble/curves`, `@noble/ciphers`, `@noble/hashes` (JS puro, auditadas, rodam no Hermes), aleatoriedade do `expo-crypto`.
- Estado: um store pequeno (`zustand`). Nenhum outro kit de estado ou de interface.
- Um development build (`expo-dev-client`), não o Expo Go: as permissões de rede local e, depois, o mDNS precisam de configuração nativa.

**PC: um módulo novo `remote.rs` em `app/`**, um plugin (`remote`, Experimental). Crates novas, cada uma necessária e pequena: features `net`/`rt` do `tokio` + `tokio-tungstenite` (servidor WebSocket), `snow` (Noise Protocol), `qrcodegen` (QR do pareamento, desenhado em SVG pela página de configurações). Depois, `mdns-sd` para descoberta. A chave privada do PC fica no cofre de credenciais do Windows pelo `keyring` (já usado pelo Last.fm).

**Pasta:** `mobile/` na raiz do repo, ao lado de `app/`. É um app separado com seu próprio `package.json`, mas o protocolo precisa mudar junto com o desktop no mesmo pull request, então um repo só. Node pelo `.nvmrc`, pnpm (`node-linker=hoisted` em `mobile/.npmrc`, que o Expo exige).

## Modelo de segurança

Noise Protocol Framework (`25519`, `ChaChaPoly`, `SHA256`) sobre WebSocket, frames binários. O Noise é um handshake publicado e analisado (WireGuard e outros usam); não inventamos criptografia própria. O desktop usa `snow`; o celular implementa os dois padrões abaixo sobre `@noble` e precisa passar nos vetores de teste oficiais do Noise (cacophony) para eles no CI antes de qualquer outro código do celular entrar.

**Chaves.** O PC tem um par de chaves X25519 estático, criado no primeiro uso. Cada celular cria o seu na instalação. Chaves privadas nunca saem do armazenamento seguro do aparelho.

**Pareamento (`Noise_IKpsk2`).**
1. O usuário liga o plugin e aperta "Parear um celular" na janela de configurações. O PC mostra por 2 minutos um QR com: versão do protocolo, endereços de LAN e porta do PC, a chave pública do PC e um segredo novo de uso único de 32 bytes. Com essa tela fechada não há pareamento possível.
2. O celular lê o QR e roda o `IKpsk2`: já conhece a chave do PC (então um PC falso na rede não consegue responder) e prova que viu o QR (o segredo é o PSK). A primeira mensagem leva sua chave pública e um nome que o usuário digitou ("Pixel 9").
3. O PC pergunta na janela de configurações: "Permitir que <nome> controle o feathertune?" (com um código de 6 dígitos também mostrado no celular, derivado do hash do handshake, então as duas telas precisam bater). Só com Permitir o PC guarda a chave pública e o nome do celular.
4. O segredo de uso único é descartado depois de um uso, de uma falha ou dos 2 minutos, o que vier primeiro.

**Toda conexão depois disso (`Noise_KK`).** Os dois lados conhecem a chave estática um do outro. O PC aceita só chaves da lista de pareados; qualquer outra é fechada após a primeira mensagem, antes de qualquer estado ser enviado. Cada conexão tem chaves efêmeras novas (sigilo futuro) e os nonces do Noise recusam repetição e reordenação.

**Outras regras.**
- Aceitar conexões só de origens privadas, link-local e loopback (RFC 1918, `fc00::/7`, `fe80::/10`, CGNAT `100.64/10` para VPNs mesh); outras são descartadas antes do handshake.
- No máximo 4 celulares pareados conectados ao mesmo tempo; handshakes limitados por endereço de origem (ex.: 5 por minuto), os ruins registrados no log sem dados.
- Limite de tamanho de mensagem (64 KB do PC, 1 KB do celular); todo comando é lido num formato fixo (`deny_unknown_fields`), números finitos e no intervalo, ids conferidos como o `player.rs` faz.
- A janela de configurações lista os celulares pareados (nome, pareado em, visto por último) com Remover; remover um fecha a conexão dele na hora. Desligar o plugin fecha a porta.
- O celular guarda a lista de PCs pareados e um bloqueio opcional (biometria). Nunca registra o conteúdo das mensagens no log.
- O que o celular vê é o que o usuário vê na janela: faixa, artista, álbum, URL da capa, posição, volume, fila (ids, títulos, artistas; no máximo 200). Sem histórico, sem dados da conta, sem estatísticas.
- `capabilities/` não muda: a página não ganha comando novo. Comandos do celular passam pelo Rust até `window.__feathertune.controls`, como a bandeja e o painel de mídia já fazem.

A seção Segurança do docs/DESIGN.md ganha um parágrafo sobre isso quando entrar.

## Protocolo

Depois do handshake, cada mensagem de transporte do Noise é um objeto JSON. Versão 1:

PC → celular:
- `{"t":"hello","v":1,"name":"<nome do PC>"}` uma vez.
- `{"t":"state","track":{id,title,artist,album,artwork,duration},"position":s,"paused":b,"ad":b,"volume":0-100,"muted":b,"colors":[...],"at":ms}`: a cada `player::Event` e ao conectar. O celular avança a posição sozinho a partir de `at` (relógio do PC, enviado com um `ping` para estimar a diferença).
- `{"t":"queue","items":[{id,title,artist}],"current":i}`: ao conectar e quando a fila muda.
- `{"t":"error","code":"ad"|"unavailable"|"invalid"}` depois de um comando recusado.

Celular → PC:
- `{"t":"cmd","c":"play"|"pause"|"next"|"previous"}`
- `{"t":"cmd","c":"seek","s":segundos}`
- `{"t":"cmd","c":"volume","v":0-100}`
- `{"t":"cmd","c":"play_item","id":"<id do vídeo>","i":índice}`: toca aquela linha da fila (os dois precisam bater, para um índice velho nunca tocar a música errada).
- `{"t":"ping","n":n}` → `{"t":"pong","n":n,"at":ms}`

## Trabalho no desktop

1. `player.rs`: acrescentar `volume`, `muted` e `ad` ao estado que a injeção `core` informa (validados como o resto).
2. `injections/controls/page.js`: acrescentar `seek(s)`, `setVolume(v)` (pelo `precise_volume` quando carregado) e `playItem(id, i)` (clica na linha da fila), cada um recusando durante anúncio, como o `volume` já faz.
3. `remote.rs`: o listener (só com o plugin ligado), os handshakes Noise, a lista de pareados no perfil (`remote-devices.json`: só nomes e chaves públicas), assinar o `Player`, traduzir comandos para `crate::player`.
4. Configurações: uma seção Remoto na aba Experimental (chave, Parear um celular com o QR, a confirmação, a lista de pareados). Textos por `t()` + `pt-BR.json`.
5. `plugins.rs` e docs/DESIGN.md: entrada do plugin, custo "ainda não medido", parágrafo de Segurança. Desligado o listener não custa nada mensurável (sem socket).
6. Firewall do Windows: o primeiro listen mostra o aviso do Windows; o app não cria regras sozinho. Documentar.

## Trabalho no celular

1. Criar `mobile/` (Expo SDK 58, TypeScript, Expo Router, dev client), lint e typecheck.
2. `mobile/src/noise/`: `IKpsk2` e `KK` sobre `@noble`, com os vetores de teste oficiais como testes. Job de CI (Ubuntu): `pnpm install --frozen-lockfile`, typecheck, lint, testes.
3. Telas de pareamento: boas-vindas, permissão de câmera, leitura, nome, código para conferir, pronto. Permissão de rede local do Android 17+ (`ACCESS_LOCAL_NETWORK`, target API 37) e do iOS (`NSLocalNetworkUsageDescription`) pedida com uma explicação antes.
4. Tocando agora: capa grande com fundo das cores do álbum, título/artista, barra de progresso (arrastar com vibração, manda um `seek` ao soltar), tocar/pausar, anterior/próxima, volume. Controles apagados com uma nota curta durante anúncio.
5. Fila: lista com a linha atual marcada; tocar numa linha toca ela (`play_item`). Puxando a partir do tocando agora, ou uma aba.
6. PCs: lista de PCs pareados, conectar/reconectar (último endereço, depois cada endereço do QR), remover. Estados claros para "PC desligado", "fora da mesma rede", "removido pelo PC" (pede para parear de novo).
7. Visual: escuro primeiro, segue o tema do sistema; um destaque tirado do álbum; aqui as animações respeitam o "reduzir movimento" do sistema (o celular não é a página do desktop, então a regra do AGENTS sobre isso não vale) e param com o app em segundo plano.

## Depois (fora da primeira versão)

- Descoberta por mDNS (`_feathertune._tcp`; PC com `mdns-sd`, celular com um módulo zeroconf) para achar um endereço novo do DHCP sem ler o QR de novo. Os endereços do QR bastam no começo.
- Curtir/não curtir, aleatório/repetir, reordenar ou remover itens da fila: cada um precisa de seletores da página em `controls` e de um motivo no DESIGN.md.
- Controles de mídia na tela de bloqueio do celular ou um widget.
- Letra no celular (o PC já tem as linhas).

## Lançamentos

Android: um APK anexado ao release do GitHub (feito com EAS ou localmente), talvez uma loja depois. iOS: EAS Build mais TestFlight exige a conta de desenvolvedor Apple do mantenedor; até lá, só Android. A versão de `mobile/` é separada da do desktop; o que precisa bater é o `v` da mensagem `hello`.

## Como ajudar

Escolha um item numerado acima, comente na issue de acompanhamento que está nele e abra um pull request por item (veja o [CONTRIBUTING.pt-BR.md](../CONTRIBUTING.pt-BR.md)). Os itens 1–2 do desktop e 1–2 do celular não dependem de nada e podem começar já. Diga o que conseguiu testar: o desktop só compila no Windows, e o que precisa do app rodando com conta logada é conferido pelo mantenedor.
