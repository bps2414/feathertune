# Escrevendo uma extensão do feathertune

Uma extensão é uma pasta que você importa em Configurações > Extensões. Ela pode mudar o visual da página do player (CSS), rodar código nela (JavaScript) e rodar código nativo numa sandbox (WebAssembly). English: [EXTENSIONS.md](EXTENSIONS.md). Como o app as roda: [DESIGN.pt-BR.md](DESIGN.pt-BR.md#extensões). Exemplos: [`examples/extensions/`](../examples/extensions/).

Extensões são código seu: o app não as revisa. Importe só o que você leu ou em que confia.

## Pacote

```
minha-extensao/
  plugin.json   o manifesto (obrigatório)
  style.css     adicionado à página do player
  page.js       roda na página do player
  plugin.wasm   roda numa sandbox WebAssembly
  icon.png      no máximo 64 KB
```

Pelo menos um de `style.css`, `page.js` e `plugin.wasm`. Importe a pasta, um `.zip` dela (stored ou deflate) ou, nos casos mais simples, um arquivo:

- um `.css` sozinho vira tema (id `local.<nome do arquivo>`);
- um `.js` sozinho tira o manifesto de um cabeçalho no topo:

```js
// ==Feathertune==
// @id seunome.meu-script
// @name Meu script
// @version 1.0.0
// @permission storage
// ==/Feathertune==
```

Limites: código de página 2 MB, WebAssembly 16 MB, o pacote 32 MB, 100 extensões.

Uma extensão importada fica desligada até você ligá-la (o diálogo lista o que ela pede). Importar, ligar e remover valem depois de reiniciar o app; as configurações valem na hora. Importar uma versão nova do mesmo id mantém o interruptor, as configurações e os dados. Erros aparecem sob a extensão na aba e no diagnóstico.

## Manifesto

```json
{
  "id": "seunome.minha-extensao",
  "name": "My extension",
  "version": "1.0.0",
  "author": "Seu nome",
  "description": "What it does, in one or two sentences.",
  "api": 1,
  "permissions": ["storage", "net:api.example.com"],
  "settings": [
    { "key": "size", "type": "number", "label": "Size", "default": 12, "min": 8, "max": 32, "step": 1 }
  ],
  "i18n": { "pt-BR": { "My extension": "Minha extensão", "Size": "Tamanho" } }
}
```

- `id`: duas ou mais partes separadas por pontos, cada uma de `a-z`, `0-9`, `_` e `-`; 3 a 64 caracteres. A pasta em que ela é importada recebe esse nome.
- `name` (1 a 64 caracteres), `version` (como `1.2.0`), `author`, `description` (até 500).
- `api`: a versão da API de extensões de que ela precisa (hoje `1`); um app com uma mais antiga a recusa.
- `i18n`: idioma (`pt-BR`) → texto em inglês → tradução, para o nome, a descrição e os rótulos e dicas das configurações.

### Permissões

| Permissão | Permite |
|---|---|
| `player.read` | Nada a mais: toda extensão lê o estado do player. Declare para dizer que lê. |
| `player.control` | Tocar, pausar, pular, buscar e mudar o volume (recusado durante anúncio, como para qualquer chamador). |
| `storage` | Guardar até 1 MB de valores JSON, só dela. |
| `notify` | Mostrar notificações do Windows, uma a cada 10 s. |
| `net:api.example.com` | Fazer requisições https para esse host. `net:*.example.com` é o host e seus subdomínios. |

### Configurações

Até 32, desenhadas na aba Extensões. Cada uma tem `key` (1 a 32 de `a-z`, `0-9`, `_`), `type`, `label`, `hint` e `default`, mais:

| `type` | Valor | Campos extras |
|---|---|---|
| `bool` | `true`/`false` | |
| `number` | um número | `min`, `max`, `step` |
| `enum` | um de `options` | `options` (1 a 50 textos) |
| `string` | texto sem caracteres de controle | `max` (tamanho, padrão 200) |
| `color` | `#rrggbb` | |

## Código de página

O `style.css` entra na página como está. O `page.js` roda depois dos módulos do próprio app, uma vez por carga da página, no mundo JavaScript da própria página, dentro de uma função que recebe `ft`, a API da extensão. Tipado em [`sdk/feathertune.d.ts`](../sdk/feathertune.d.ts).

```js
ft.on('track', (state) => ft.log('tocando', state.title, 'de', state.artist));
```

- `ft.id`, `ft.version`, `ft.api`, `ft.permissions`, `ft.settings` (os valores de agora), `ft.onSettings(fn)`.
- `ft.on(evento, fn)` devolve uma função que remove `fn`. Eventos:
  - `state`: qualquer mudança do estado do player; `track`: uma faixa nova; `play`, `pause`, `seek`; `ad`: `true` quando um anúncio começa, `false` quando acaba;
  - `queue`: `{queue, current, next, list}`;
  - `lyrics`, `colors`: a letra sincronizada e as cores do álbum, quando os plugins Letra sincronizada e Cor do álbum do próprio app estão ligados;
  - `settings`: os valores novos; `native`: uma mensagem do WebAssembly dela.
- `ft.player.state()`: `{id, title, artist, album, artwork, duration, position, paused, volume, muted, ad, list}` ou `null`. `ft.player.queue()`, `ft.player.video()` (o `<video>`), `ft.player.clock` (`time(video)`, a posição exata).
- Com `player.control`: `ft.player.play()`, `pause()`, `playPause()`, `next()`, `previous()`, `seek(segundos)`, `setVolume(0..100)`, `volumeUp()`, `volumeDown()`; cada um devolve `false` quando recusado.
- `ft.theme.colors()`, `ft.lyrics.current()`.
- `ft.addStyle(css)` devolve o `<style>`. `ft.log(...)` vai para o console e o diagnóstico (30 linhas por minuto); `ft.error(e)` mostra `e` na aba Extensões.
- `ft.storage.get(key)`, `set(key, value)`, `remove(key)`, `keys()`: promises (`storage`).
- `ft.fetch(url, {method, headers, body})` → uma promise de `{status, headers, body}` (body como texto). Só https, para os hosts `net:` dela, feita pelo app (então CORS não se aplica): sem redirecionamento, sem cookies, 10 s, 1 MB de ida, 2 MB de volta, 60 por minuto.
- `ft.notify(title, body)`: uma promise (`notify`).
- `ft.native.available`, `ft.native.send(data)`: uma mensagem para o WebAssembly dela (256 KB).

Erros não tratados nos callbacks dela são capturados e aparecem na aba; uma que nunca começou (erro de sintaxe) é avisada como não iniciada.

## WebAssembly

O `plugin.wasm` roda num interpretador numa thread própria, sem WASI: não alcança nada além das funções abaixo. Rust: [`sdk/rust`](../sdk/rust) (o crate `feathertune-ext`) embrulha tudo isto; veja [`examples/extensions/track-counter`](../examples/extensions/track-counter).

```
cargo build --release --target wasm32-unknown-unknown
```

depois copie `target/wasm32-unknown-unknown/release/<nome>.wasm` para `plugin.wasm`, ao lado do `plugin.json`.

Qualquer linguagem serve se o módulo tiver esta ABI (todo JSON em UTF-8):

- Exports: `memory`, `ft_alloc(len: i32) -> i32` (memória para o app escrever um evento) e `ft_event(ptr: i32, len: i32)` (um evento; o módulo o libera).
- Imports (módulo `ft`):
  - `call(ptr: i32, len: i32) -> i32`: um pedido `{"op": ..., ...}`; devolve o tamanho da resposta, `{"ok": valor}` ou `{"error": "texto"}`;
  - `take(ptr: i32, cap: i32) -> i32`: copia essa resposta para a memória e devolve os bytes copiados;
  - `log(ptr: i32, len: i32)`: uma linha no diagnóstico.
- Eventos (`type`): `init` (`id`, `version`, `settings`, `app_version`), `track` e `state` (`state`), `ad` (`ad`, `state`), `settings` (`settings`), `page` (`data`, do código de página dela), `timer` (`id`).
- Pedidos (`op`): `state`, `queue`, `settings`, `now` (ms desde 1970), `command` (`name`: play, pause, playPause, next, previous, seek, volume; `value` para os dois últimos; `player.control`), `timer` (`ms` de 10 a 86 400 000, `id`; no máximo 16 esperando), `page_send` (`data`, para `ft.on('native')`), `storage_get`/`storage_set`/`storage_remove`/`storage_keys` (`key`, `value`), `fetch` (`url`, `method`, `headers`, `body`), `notify` (`title`, `body`), `log` e `error` (`message`).

Limites: 64 MB de memória; o início e cada evento podem rodar cerca de 2·10⁹ instruções. Um trap, passar disso ou um ponteiro inválido param o módulo até o app reiniciar, e o erro aparece na aba.
