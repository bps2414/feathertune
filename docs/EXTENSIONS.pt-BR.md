# Escrevendo uma extensão do feathertune

Uma extensão é uma pasta que você importa em Configurações > Extensões. English: [EXTENSIONS.md](EXTENSIONS.md).

```
minha-extensao/
  plugin.json   o manifesto (obrigatório)
  page.js       roda na página do player (opcional)
  style.css     entra na página do player (opcional)
  icon.png      até 64 KB (opcional)
```

`plugin.json`:

```json
{
  "id": "seunome.minha-extensao",
  "name": "Minha extensão",
  "version": "1.0.0",
  "author": "Seu nome",
  "description": "O que ela faz, em uma ou duas frases.",
  "api": 1
}
```

- `id`: duas ou mais partes separadas por pontos, cada uma de `a-z`, `0-9`, `_` e `-`; de 3 a 64 caracteres. A pasta em que ela é importada recebe esse nome.
- `api`: a versão da API de extensões de que ela precisa; um app com uma mais antiga a recusa.

Um `style.css` sozinho pode ser importado como tema. Importe, ligue e reinicie o app.
