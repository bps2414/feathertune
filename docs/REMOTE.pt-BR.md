# Controle remoto pelo celular: plano

English: [REMOTE.md](REMOTE.md)

Um app de celular que controla o feathertune num PC pareado pela rede local. Ele não toca música: mostra o que o PC toca (faixa, capa, posição, fila) e manda tocar/pausar, próxima, anterior, buscar posição, volume e "tocar este item da fila". Depois, o mesmo protocolo liga vários PCs com feathertune e celulares numa pequena rede de aparelhos (um seletor de aparelhos, um PC controlando outro, levar a reprodução de um PC para outro). Ainda não começou; este é o plano a partir do qual quem contribuir deve construir. Dúvidas e andamento vão na issue de acompanhamento.

## Isso já existe?

Conferido em outubro de 2026, para não refazermos algo que o serviço ou o PC já dão:

- **Os apps do próprio serviço não fazem isso.** O app do celular transmite para TVs, caixas de som e streamers e vira o controle deles, mas o navegador de um computador é sempre só *quem transmite*, nunca o destino, e nada no celular lista ou controla uma aba do navegador. Desde meados de 2025 o serviço sincroniza a fila e a posição entre o celular e a web ("continue de onde parou"; desde o começo de 2026 entre celulares também): é uma passagem que você toca no aparelho para onde vai, não controle ao vivo e nem um seletor de aparelhos. O feathertune hospeda o site oficial, então esse "continuar" já funciona nele, nos dois sentidos; não vamos refazê-lo.
- **Um stream por conta.** Uma conta paga toca música em um aparelho por vez: começar num segundo pausa o primeiro ("conta em uso em outro local"). Tocar a mesma conta em dois cômodos ao mesmo tempo, portanto, está fora, e nada aqui tenta contornar isso.
- **Os botões básicos já funcionam com um controle genérico.** O feathertune tem a própria entrada na sessão de mídia do Windows (`smtc.rs`), então um app de celular que controla a sessão de mídia do PC já tem tocar/pausar, próxima, anterior, buscar posição e o volume do sistema. Esses apps existem (alguns comerciais, com anúncios ou plano pago). Eles não mostram nem tocam a fila, não distinguem um anúncio de uma faixa, não mexem no volume do próprio player e não mostram as cores do álbum nem a letra.
- **Outros clientes desktop não oficiais têm controle remoto.** Os dois maiores servem HTTP ou WebSocket puro em todas as interfaces (`0.0.0.0`), aprovam um cliente uma vez com um popup ou um código e depois confiam num token enviado às claras pela LAN; quem estiver na Wi-Fi e enxergar esse token controla o player.

Então o controle remoto vale a pena pela fila, por saber de anúncios, pelo visual, por vários PCs e por um modelo de segurança mais forte; os botões básicos são consequência, não o motivo.

## Objetivos e limites

- **Só aparelhos pareados controlam um PC, e só o PC com que parearam.** Um celular na mesma Wi-Fi que nunca foi pareado, um código de pareamento roubado depois de usado, uma mensagem repetida ou um aparelho que o usuário removeu não recebem nada: nem estado, nem comandos.
- Só rede local. Sem relay na nuvem, sem conta, sem redirecionamento de porta, sem UPnP. Uso fora de casa só pela VPN do próprio usuário (ex.: uma VPN mesh), que parece LAN para os dois lados.
- Desligado por padrão (aba Experimental), como todo recurso que abre algo para fora.
- As regras do [AGENTS.md](../AGENTS.md) valem sem mudança: nada durante um anúncio é pulado, encurtado ou silenciado (buscar posição, próxima, anterior, saltos na fila e levar a reprodução para outro aparelho são recusados enquanto um toca, e o volume não muda); nenhum áudio sai do PC; nada de nomes de marca no nome do app, na loja ou na interface.
- Nunca transmitir para TVs ou caixas de som e nunca controlar os apps do serviço em outros aparelhos: os dois exigem os protocolos privados de pareamento e transmissão do serviço (nada de APIs privadas), e o WebView2 já tem a descoberta de transmissão desligada (docs/DESIGN.pt-BR.md, −12 MB). Para a TV, o usuário transmite pelo app do serviço no celular, e a fila sincronizada dele continua de onde o PC parou.
- Nunca tocar em dois aparelhos ao mesmo tempo (veja "Um stream por conta" acima).

## Rede de aparelhos

O que outros serviços chamam de "connect": cada aparelho do usuário vê os outros e qualquer um deles comanda o que está tocando. O que cabe nos limites acima:

- **Players** são PCs com feathertune. **Controles** são celulares e, depois, outros PCs com feathertune. Um PC é os dois.
- Cada player tem um **id de aparelho**: os primeiros 8 bytes do SHA-256 da sua chave pública estática, em hex. Estável enquanto a chave for, nunca o nome do computador no Windows.
- Um controle guarda uma entrada por player pareado (id, nome dado pelo usuário, chave pública, últimos endereços). O pareamento é por par de aparelhos; não há chave de grupo compartilhada e nenhum aparelho confia em outro só porque um terceiro confia.
- **Descoberta** (etapa 2) por mDNS (`_feathertune._tcp`). O anúncio não pode identificar o PC para estranhos: sem nome e sem id fixo, só a porta, a versão do protocolo e uma marca que só aparelhos pareados reconhecem (um HMAC da hora atual com uma chave trocada no pareamento). Anunciado só com o plugin ligado.
- **Seletor de aparelhos** (etapa 2): o celular lista os PCs pareados com ligado/desligado e o que cada um toca, por uma conexão curta com cada um enquanto o seletor está aberto; fica conectado só ao escolhido.
- **Levar a reprodução de um PC para outro** (etapa 3): pelo seletor, "tocar aqui" no PC B pausa primeiro o PC A, depois o B abre a mesma faixa, na mesma posição, na mesma playlist (`watch?v=<id>&list=<list>` no site oficial, depois um seek quando começa a tocar; um anúncio que vier antes toca inteiro). É só uma navegação no site oficial. Fazer só se o "continuar" do próprio serviço se mostrar lento ou manual demais na prática; conferir isso antes.
- **Continuar no celular** (etapa 3): pausa o PC e abre a faixa na posição dela no app do serviço no celular por um link normal. Toca lá sob as regras do serviço; o nosso app continua sem tocar nada.
- **Fora, e por quê:** TVs, caixas de som e outros navegadores como destino (protocolos privados); vários cômodos sincronizados numa conta (um stream por conta); um relay na nuvem como outros serviços usam para alcançar aparelhos em outras redes (servidor nosso, contas e superfície de ataque; a VPN do usuário faz isso no lugar).

## Stack

**Celular: Expo (React Native), SDK 58, TypeScript.** Escolhido em vez das alternativas porque:

| Opção | Por que não (para este app) |
|---|---|
| Tauri 2 mobile | Compartilharia Rust com o desktop, mas a interface é um webview, o mobile é o lado menos maduro do Tauri (nem todos os plugins portados) e um build de iOS exige Xcode num Mac. O mantenedor usa só Windows. |
| Flutter | Boa interface, mas uma terceira linguagem (Dart) sem nada compartilhado, e builds de iOS também exigem Mac ou CI pago. |
| Nativo (Kotlin + Swift) | Duas bases de código para um app pequeno. |
| Interface em Rust (Dioxus, Slint) | Suporte a mobile ainda novo; toque, gestos, câmera e armazenamento seguro seriam feitos à mão. |

O Expo dá interface nativa com gestos e animações suaves, câmera (ler QR), Keychain/Keystore e vibração como módulos mantidos, e o EAS Build compila iOS na nuvem sem Mac. Bibliotecas:

- `expo-router` (telas), o `PanResponder` e o `Animated` do próprio React Native (barra de progresso, volume, painel da fila; por enquanto sem biblioteca de gestos ou de animação), `expo-image` (capa com fundo desfocado), `expo-linear-gradient` com as cores do álbum que o PC já calcula (`album_color.rs`), `expo-splash-screen` (a tela de abertura, feita do ícone do app do PC).
- `expo-camera` (leitor de QR), `expo-secure-store` (chaves), `expo-haptics`, `expo-local-authentication` (bloqueio opcional do app), `expo-localization` (inglês e pt-BR, como o desktop).
- Criptografia: `@noble/curves`, `@noble/ciphers`, `@noble/hashes` (JS puro, auditadas, rodam no Hermes), aleatoriedade do `expo-crypto`.
- Estado: um store pequeno (`zustand`). Nenhum outro kit de estado ou de interface.
- Um development build (`expo-dev-client`), não o Expo Go: as permissões de rede local e, depois, o mDNS precisam de configuração nativa.

**PC: um módulo novo `remote.rs` em `app/`**, um plugin (`remote`, Experimental). O `tokio` já é dependência (`sync`, `time`); ganha `net`. Crates novas, cada uma necessária e pequena: `tokio-tungstenite` (servidor WebSocket), `snow` (Noise Protocol; só 25519, ChaChaPoly e SHA256), `futures-util` (os traits `Stream`/`Sink` do socket; já na árvore pelo `tokio-tungstenite`), `qrcodegen` (QR do pareamento, desenhado em SVG pela página de configurações). A etapa 2 acrescenta `mdns-sd`. A chave privada do PC fica no cofre de credenciais do Windows pelo `keyring` (já usado pelo Last.fm).

**Pasta:** `mobile/` na raiz do repo, ao lado de `app/`. É um app separado com seu próprio `package.json`, mas o protocolo precisa mudar junto com o desktop no mesmo pull request, então um repo só. Node pelo `.nvmrc`, pnpm (`node-linker=hoisted` em `mobile/.npmrc`, que o Expo exige).

## Modelo de segurança

Noise Protocol Framework (`25519`, `ChaChaPoly`, `SHA256`) sobre WebSocket, frames binários. O Noise é um handshake publicado e analisado (WireGuard e outros usam); não inventamos criptografia própria. O desktop usa `snow`; o celular implementa os dois padrões abaixo sobre `@noble` e precisa passar nos vetores de teste oficiais do Noise (cacophony) para eles no CI antes de qualquer outro código do celular entrar.

**Chaves.** O PC tem um par de chaves X25519 estático, criado no primeiro uso. Cada celular cria o seu na instalação. Chaves privadas nunca saem do armazenamento seguro do aparelho. "Esquecer todos os aparelhos" nas configurações cria um par de chaves novo para o PC, então todo pareamento antigo (e qualquer cópia dele) para de funcionar na hora.

**Pareamento (`Noise_IKpsk2`).**
1. O usuário liga o plugin e aperta "Parear um celular" na janela de configurações. O PC mostra por 2 minutos um QR com: versão do protocolo, endereços de LAN e porta do PC, a chave pública do PC e um segredo novo de uso único de 32 bytes. Com essa tela fechada não há pareamento possível.
2. O celular lê o QR e roda o `IKpsk2`: já conhece a chave do PC (então um PC falso na rede não consegue responder) e prova que viu o QR (o segredo é o PSK). A primeira mensagem leva sua chave pública e um nome que o usuário digitou ("Pixel 9").
3. O PC pergunta na janela de configurações: "Permitir que <nome> controle o feathertune?" (com um código de 6 dígitos também mostrado no celular, derivado do hash do handshake, então as duas telas precisam bater). Só com Permitir o PC guarda a chave pública e o nome do celular.
4. O segredo de uso único é descartado depois de um uso, de uma falha ou dos 2 minutos, o que vier primeiro. Bytes que não são uma mensagem 1 para este PC (qualquer um na rede pode mandá-los) não contam como tentativa, então não fecham a janela.
5. O PC manda seu nome (o que o usuário definiu na seção Remoto; por padrão "feathertune", nunca o nome do computador no Windows, que muitas vezes tem o nome de uma pessoa) e, a partir da etapa 2, a chave de descoberta.

Um PC não lê QR, então parear um PC com outro (etapa 3) precisa de outro primeiro passo: o PC de destino mostra um código de uso único de pelo menos 80 bits (16 caracteres base32, digitados no outro PC), achado por mDNS, usado como PSK, com a mesma conferência de 6 dígitos e os mesmos 2 minutos. A revisar antes de ser feito.

**Toda conexão depois disso (`Noise_KK`).** Os dois lados conhecem a chave estática um do outro. O PC aceita só chaves da lista de pareados; qualquer outra é fechada após a primeira mensagem, antes de qualquer estado ser enviado. Cada conexão tem chaves efêmeras novas (sigilo futuro) e os nonces do Noise recusam repetição e reordenação.

**Outras regras.**
- Aceitar conexões só de origens privadas, link-local e loopback (RFC 1918, `fc00::/7`, `fe80::/10`, CGNAT `100.64/10` para VPNs mesh); outras são descartadas antes do handshake.
- No máximo 4 aparelhos pareados conectados ao mesmo tempo; handshakes limitados por endereço de origem (ex.: 5 por minuto), os ruins registrados no log sem dados.
- Limite de tamanho de mensagem (64 KB do PC, 1 KB de um controle); todo comando é lido num formato fixo (`deny_unknown_fields`), números finitos e no intervalo, ids conferidos como o `player.rs` faz.
- A janela de configurações lista os aparelhos pareados (nome, pareado em, visto por último) com Remover; remover um fecha a conexão dele na hora. Desligar o plugin fecha a porta e para o anúncio mDNS.
- O celular guarda a lista de PCs pareados e um bloqueio opcional (biometria). Nunca registra o conteúdo das mensagens no log.
- O que um controle vê é o que o usuário vê na janela: faixa, artista, álbum, URL da capa, posição, volume, fila (ids, títulos, artistas; no máximo 200), os nomes e ids das playlists da barra lateral do site (no máximo 100). Sem histórico, sem dados da conta, sem estatísticas.
- `capabilities/` não muda: a página não ganha comando novo. Comandos de um controle passam pelo Rust até `window.__feathertune.controls`, como a bandeja e o painel de mídia já fazem.

A seção Segurança do docs/DESIGN.md ganha um parágrafo sobre isso quando entrar.

## Protocolo

Tudo o que um controle precisa para falar com o PC (`app/src/remote.rs`; os testes dele rodam um iniciador `snow` por cada passo abaixo). Versão 2, que acrescentou `library` e os comandos de fila e de playlist; um controle da versão 1 continua funcionando com ela (`min` é 1). O `v` do QR, os caminhos e os prologues não mudaram.

**Transporte.**
- WebSocket sobre TCP, `ws://<endereço>:<porta>`, porta padrão **47652** (`[remote] port` no `config.toml`). Não é `wss`: a criptografia é o Noise. Caminhos: `/v1/pair` para parear, `/v1` para toda sessão depois; qualquer outro caminho recebe HTTP 404.
- Um frame binário é uma mensagem Noise (no máximo 65 535 bytes). Um frame de texto fecha a conexão (`invalid`). Um frame do controle tem no máximo 1 040 bytes (1 024 de JSON mais a tag de 16 bytes); um maior fecha a conexão (`too_big`). Mensagens do PC têm no máximo 65 519 bytes de JSON.
- O PC descarta, antes do handshake do WebSocket (a conexão TCP só fecha), origens fora de `10/8`, `172.16/12`, `192.168/16`, `169.254/16`, `127/8`, `100.64/10`, `::1`, `fc00::/7`, `fe80::/10` (IPv6 mapeado de IPv4 conta como IPv4), e uma sexta conexão do mesmo endereço em um minuto.
- Cada passo espera no máximo 10 s: o upgrade do WebSocket, a primeira mensagem Noise, o primeiro `ping` de uma sessão. A exceção é a prova do pareamento (passo 4 abaixo), esperada até os 2 minutos acabarem (aí `timeout`).
- Frames de fechamento vão em claro (são frames do WebSocket, não mensagens Noise); os códigos estão abaixo.

**QR de pareamento.** Uma URI, hex minúsculo:

```
feathertune://pair?v=1&k=<chave pública estática do PC, 64 hex>&s=<segredo de uso único, 64 hex>&a=<endereços IPv4, separados por vírgula>&p=<porta>
```

O controle ignora parâmetros que não conhece. `a` traz o endereço da interface do PC na rota padrão (um, hoje). O id do aparelho do PC são os 8 primeiros bytes do SHA-256 dos 32 bytes de `k`, em hex.

**Pareamento, `/v1/pair`.** `Noise_IKpsk2_25519_ChaChaPoly_SHA256`; o celular é o iniciador, a chave estática do respondedor é `k`, a PSK é `s` (posição 2) e o prologue são os 25 bytes ASCII `feathertune-remote/1 pair`.
1. Celular → PC, mensagem 1 do handshake (`e, es, s, ss`). O payload é o JSON UTF-8 `{"name":"<nome do celular>"}`, só essa chave; o nome tem de 1 a 32 caracteres depois de aparado, sem caracteres de controle.
2. PC → celular, mensagem 2 do handshake (`e, ee, se, psk`), payload vazio. Um celular que não consegue decifrá-la está falando com algo que não tem o segredo: para.
3. Os dois calculam o **código de 6 dígitos**: os 4 primeiros bytes do hash do handshake (`h` depois da mensagem 2, o `get_handshake_hash` do snow) como inteiro sem sinal de 32 bits big-endian, módulo 1 000 000, completado com zeros até 6 dígitos. O celular o mostra.
4. Celular → PC, a primeira mensagem de transporte: `{"t":"pair"}`. Ela prova que o celular usou o segredo (a PSK só entra com a mensagem 2), então só agora o PC mostra "Permitir <nome>?" com o mesmo código.
5. Permitir: o PC guarda o celular, e a conexão segue como sessão (abaixo), começando pelo `hello` (nenhum `ping` veio antes, então não há `pong`); o celular guarda o PC (`k`, o nome do `hello`, o endereço que usou) quando esse `hello` chega. Negar: fecha com `denied`. Os 2 minutos acabando ou a tela das configurações fechando: `expired`.

O segredo vale uma vez: a primeira mensagem 1 que o PC consegue ler (feita com `k`) o consome, aconteça o que acontecer depois (um segundo celular, ou o mesmo de novo, recebe `expired`). Uma que ele não consegue ler é fechada com `invalid` e deixa a janela aberta.

**Sessão, `/v1`.** `Noise_KK_25519_ChaChaPoly_SHA256`; o celular é o iniciador com o próprio par de chaves e o `k` do PC, e o prologue são os 20 bytes ASCII `feathertune-remote/1`. A mensagem 1 (`e, es, ss`) e a mensagem 2 (`e, ee, se`) têm payload vazio. O PC testa a mensagem 1 contra cada chave pareada; sem nenhuma, fecha com `unknown`, sem responder. Depois da mensagem 2, a primeira mensagem de transporte do celular precisa ser um `ping`; só então o PC manda `hello`, `state`, `queue`, `library` e o `pong`, nessa ordem (uma mensagem 1 repetida recebe a mensagem 2 e nada mais). No máximo 4 sessões ao mesmo tempo (`busy`); o mesmo celular conectando de novo substitui a sessão antiga, que é derrubada sem frame de fechamento.

Depois do handshake cada mensagem de transporte do Noise é um objeto JSON, lido num formato fixo: campos desconhecidos, tipos errados e valores fora da faixa são recusados.

PC → controle:
- `{"t":"hello","v":2,"min":1,"name":"<nome do PC>"}` uma vez. Um controle fora de `min`..`v` fecha e diz qual lado atualizar.
- `{"t":"state","track":{"id","title","artist","album","artwork","duration"},"position":s,"paused":b,"ad":b,"volume":0-100,"muted":b,"colors":[...],"at":ms}`: a cada `player::Event`, ao conectar (depois que algo tocou) e quando as cores do álbum chegam. `id` (o id de 11 caracteres do vídeo) e `artwork` (uma URL https) podem ser `null`; `duration` em segundos, 0 enquanto desconhecida. `position` em segundos no instante `at` (milissegundos Unix, relógio do PC): o controle avança sozinho enquanto `paused` é falso (a diferença de relógio é estimada pelo `ping`/`pong`). `colors` é `["#destaque","#tinta","#texto no destaque"]` da capa, ou `[]`; depois de uma troca de faixa elas vêm num `state` seguinte.
- `{"t":"queue","items":[{"id","title","artist"}],"current":i}`: ao conectar e quando a fila muda. No máximo 200 itens, cortados do fim se a mensagem não couber; `current` é `null` quando não se sabe.
- `{"t":"library","items":[{"list","title"}]}`: ao conectar e quando muda, as playlists da barra lateral do site (a biblioteca do usuário), lidas pela injeção `core` mesmo antes de tocar algo. No máximo 100; `list` é um id de playlist, `title` com no máximo 200 caracteres. Vazia enquanto a barra lateral está fechada ou ainda não carregou.
- `{"t":"error","code":"ad"|"unavailable"|"invalid"}` depois de uma mensagem recusada: `ad` enquanto toca um anúncio, `unavailable` quando a página não pôde ser alcançada, `invalid` para qualquer coisa malformada. Um comando que deu certo não tem resposta; o próximo `state` mostra o efeito.

Controle → PC:
- `{"t":"cmd","c":"play"|"pause"|"next"|"previous"}`
- `{"t":"cmd","c":"seek","s":segundos}`: de 0 a 86 400.
- `{"t":"cmd","c":"volume","v":0-100}`
- `{"t":"cmd","c":"play_item","id":"<id do vídeo>","i":índice}`: toca aquela linha da fila (os dois precisam bater, para um índice velho nunca tocar a música errada); `i` de 0 a 199.
- `{"t":"cmd","c":"queue_next"|"queue_remove","id","i"}` (versão 2): aquela linha logo depois da que toca, ou fora da fila, pelo próprio menu da linha no site. Mesmas checagens do `play_item`. O "Tocar a seguir" do site cria uma cópia, então o `queue_next` depois remove a linha de origem: a linha se move. Não existe `queue_move` (a página ignorava um arrasto que não veio de um mouse); o PC responde `invalid` a ele.
- `{"t":"cmd","c":"open_playlist","list":"<id da playlist>"}` (versão 2): toca aquela playlist do começo, uma navegação do site oficial (`/watch?list=`).
- `{"t":"ping","n":n}` → `{"t":"pong","n":n,"at":ms}`: primeira coisa da sessão, depois a cada 15 s; o PC fecha uma conexão calada por 45 s. `n` é um inteiro sem sinal.

Durante um anúncio todo comando exceto `play` e `pause` é recusado com `ad` (pausar não encurta nada); um comando malformado continua recebendo `invalid`, pois é conferido antes.

Códigos de fechamento (o motivo é a palavra entre crases):

| Código | Motivo | Quando | Celular |
|---|---|---|---|
| 4000 | `denied` | o usuário apertou Negar | avisa; parear de novo para tentar outra vez |
| 4001 | `expired` | nenhuma tela de pareamento aberta, os 2 minutos acabaram ou o segredo já foi usado | ler um QR novo |
| 4002 | `unknown` | `KK` com uma chave que o PC não tem (removida, ou Esquecer todos os aparelhos), ou uma primeira mensagem que nenhuma chave pareada lê | "removido pelo PC": parear de novo |
| 4003 | `removed` | o usuário removeu este celular (ou esqueceu todos) com ele conectado | "removido pelo PC": parear de novo |
| 4004 | `busy` | já há 4 sessões abertas | tentar mais tarde |
| 4005 | `timeout` | 45 s sem mensagem (10 s durante um handshake) | reconectar |
| 4006 | `invalid` | uma mensagem que não decifra (repetida, fora de ordem, forjada), um frame de texto, um handshake ruim | reconectar; relatar se repetir |
| 4007 | `too_big` | um frame acima de 1 040 bytes | bug no controle |
| 4008 | `off` | o plugin foi desligado | "o controle remoto está desligado no PC" |
| 4009 | `full` | já há 16 celulares pareados | remover um no PC |

Vários controles ao mesmo tempo recebem todo `state`; comandos valem na ordem de chegada (o último vence). A etapa 3 acrescenta `{"t":"cmd","c":"hand_off"}` (o PC A → sua posição pausada e a playlist para o PC B) e `{"t":"cmd","c":"open","id","list","s"}` (o PC B abre), os dois recusados durante anúncio, e o `v` vira 3.

## Ciclo de vida

- **PC:** o listener roda enquanto o plugin está ligado e o app aberto, inclusive com a janela escondida ou na bandeja (é quando um controle remoto mais serve). Quando o Windows entra em suspensão, as conexões caem e os controles mostram o PC desligado. Acordá-lo pelo celular (Wake-on-LAN, com o endereço MAC enviado no pareamento) é uma ideia para depois, não da etapa 1.
- **Celular:** conectado só com o app em primeiro plano; fecha o socket alguns segundos depois de ir para segundo plano, então não gasta bateria parado. Controles na tela de bloqueio e na notificação (etapa 2) exigem um foreground service no Android enquanto um PC toca; o iOS não mantém socket em segundo plano, então lá fica só em primeiro plano.
- **Reconectar:** último endereço que funcionou, depois a resposta do mDNS (etapa 2), depois cada endereço do QR; espera 1, 2, 4… até 30 s, só em primeiro plano.
- **Firewall do Windows:** o primeiro listen mostra o aviso do Windows; o app não cria regras. Numa rede que o Windows marca como Pública, conexões de entrada são bloqueadas: a seção Remoto diz isso e como marcar a rede como Privada.

## Trabalho no desktop

1. `player.rs`: acrescentar `volume`, `muted` e `ad` ao estado que a injeção `core` informa (validados como o resto). Guardar a última fila e o índice atual no `Player` e mandar um evento quando ela muda: hoje a fila vai só para o `mixes::observe`, e a injeção `core` só a manda quando a URL tem playlist (`list=`); o remoto precisa dela também em rádio e reprodução automática.
2. `injections/controls/page.js`: acrescentar `seek(s)`, `setVolume(v)` (pelo `precise_volume` quando carregado) e `playItem(id, i)` (clica na linha da fila), cada um recusando durante anúncio, como o `volume` já faz.
3. `remote.rs`: o listener (só com o plugin ligado), os handshakes Noise, a lista de pareados no perfil (`remote-devices.json`: só nomes e chaves públicas), assinar o `Player`, traduzir comandos para `crate::player`. Os testes dele rodam um iniciador `snow` contra ele no mesmo processo: pareamento, `KK`, chave revogada, chave desconhecida, repetição, comando grande demais e malformado. Isso cobre o lado do PC no CI sem celular.
4. Configurações: uma seção Remoto na aba Experimental (chave, nome do PC, Parear um celular com o QR, a confirmação, a lista de pareados, Esquecer todos os aparelhos, o aviso de rede Pública). Textos por `t()` + `pt-BR.json`.
5. `plugins.rs` e docs/DESIGN.md: entrada do plugin, custo "ainda não medido", parágrafo de Segurança. Desligado o listener não custa nada mensurável (sem socket).

## Trabalho no celular

1. Criar `mobile/` (Expo SDK 58, TypeScript, Expo Router, dev client), lint e typecheck.
2. `mobile/src/noise/`: `IKpsk2` e `KK` sobre `@noble`, com os vetores de teste oficiais como testes. Job de CI (Ubuntu): `pnpm install --frozen-lockfile`, typecheck, lint, testes.
3. `mobile/tools/fake-pc`: um script Node que faz o papel do PC (QR de pareamento no terminal, `KK`, uma faixa e uma fila inventadas que andam, recusas durante um anúncio de mentira) com o mesmo código de `noise/`. Assim o trabalho no celular não precisa de PC com Windows nem de conta, e quem contribui em qualquer sistema consegue fazer todas as telas.
4. Telas de pareamento: boas-vindas, permissão de câmera, leitura, nome, código para conferir, pronto. Permissão de rede local do Android 17+ (`ACCESS_LOCAL_NETWORK`, target API 37) e do iOS (`NSLocalNetworkUsageDescription`) pedida com uma explicação antes.
5. Tocando agora: capa grande com fundo das cores do álbum, título/artista, barra de progresso (arrastar com vibração, manda um `seek` ao soltar), tocar/pausar, anterior/próxima, volume (segue o dedo: no máximo um `volume` a cada 80 ms enquanto arrasta, sempre o último valor). Controles apagados com uma nota curta durante anúncio.
6. Fila: um painel sobre o tocando agora, aberto pela barra "a seguir"; a linha atual marcada, tocar numa linha toca ela (`play_item`), e o menu de cada linha tem "Tocar a seguir" e "Remover". Uma aba Biblioteca ao lado do player lista as playlists da barra lateral; tocar numa toca ela (`open_playlist`). Com um PC da versão 1 elas pedem para atualizar o feathertune.
7. PCs: lista de PCs pareados, conectar/reconectar, remover. Estados claros para "PC desligado ou em suspensão", "fora da mesma rede", "o Windows está bloqueando a conexão (rede Pública)", "removido pelo PC" (pede para parear de novo), "atualize o app" / "atualize o feathertune".
8. Visual: escuro primeiro, segue o tema do sistema; um destaque tirado do álbum; aqui as animações respeitam o "reduzir movimento" do sistema (o celular não é a página do desktop, então a regra do AGENTS sobre isso não vale) e param com o app em segundo plano. Todo controle tem rótulo de acessibilidade e o texto segue o tamanho de fonte do sistema.

## Etapas

1. **Celular → um PC** (tudo acima): parear, tocando agora, fila (com reordenar e remover), as playlists da biblioteca, vários PCs pareados trocados à mão.
2. **Achar e escolher:** descoberta por mDNS, o seletor de aparelhos com o estado de cada PC, controles na tela de bloqueio e na notificação, letra no celular (o PC já tem as linhas), curtir/não curtir e aleatório/repetir (cada um precisa de seletores da página em `controls` e de um motivo no DESIGN.md).
3. **PCs juntos:** um PC com feathertune controlando outro (mesmo protocolo, um cliente `snow` no `remote.rs`, um seletor no menu da bandeja), levar a reprodução de um PC para outro, continuar no celular. Wake-on-LAN se pedirem.

Cada etapa sai sozinha e mantém as anteriores funcionando; o `min` do `hello` mantém um app de celular mais antigo utilizável enquanto os comandos que ele conhece existirem.

## Perguntas em aberto

Para o mantenedor, com o app rodando e uma conta logada:
- O "continuar" do próprio serviço já leva a reprodução entre dois PCs com feathertune bem o bastante? Se sim, a passagem da etapa 3 sai.
- Um seek logo depois de abrir uma faixa pela URL cai no lugar certo sempre (e depois de um anúncio)?
- O `playItem` clicando numa linha da fila funciona em filas de reprodução automática e de rádio, não só em playlists?
- O `queueNext` e o `queueRemove` acham as entradas do menu da linha, e a injeção `core` acha as playlists da barra lateral (barra fechada, biblioteca longa)?

## Lançamentos

Android: um APK anexado ao release do GitHub (feito pelo job `android` do CI a cada tag `v*`, assinado sempre com a mesma chave de debug para instalar por cima do anterior), talvez uma loja depois. iOS: EAS Build mais TestFlight exige a conta de desenvolvedor Apple do mantenedor; até lá, só Android. A versão de `mobile/` é separada da do desktop; o que precisa bater é a faixa `min`..`v` da mensagem `hello`.

## Como ajudar

Escolha um item numerado acima, comente na issue de acompanhamento que está nele e abra um pull request por item (veja o [CONTRIBUTING.pt-BR.md](../CONTRIBUTING.pt-BR.md)). Os itens 1–2 do desktop e 1–3 do celular não dependem de nada e podem começar já. Diga o que conseguiu testar: o desktop só compila no Windows, e o que precisa do app rodando com conta logada é conferido pelo mantenedor.
