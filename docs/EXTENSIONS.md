# Writing a feathertune extension

An extension is a folder you import in Settings > Extensions. pt-BR: [EXTENSIONS.pt-BR.md](EXTENSIONS.pt-BR.md).

```
my-extension/
  plugin.json   the manifest (required)
  page.js       runs in the player page (optional)
  style.css     added to the player page (optional)
  icon.png      at most 64 KB (optional)
```

`plugin.json`:

```json
{
  "id": "yourname.my-extension",
  "name": "My extension",
  "version": "1.0.0",
  "author": "Your name",
  "description": "What it does, in one or two sentences.",
  "api": 1
}
```

- `id`: two or more parts separated by dots, each of `a-z`, `0-9`, `_` and `-`; 3 to 64 characters. The folder it is imported into takes this name.
- `api`: the version of the extension API it needs; an app with an older one refuses it.

A single `style.css` can be imported alone as a theme. Import, switch on and restart the app.
