export const locales = ['en', 'pt-br'] as const;
export type Locale = (typeof locales)[number];

export const htmlLang: Record<Locale, string> = { en: 'en', 'pt-br': 'pt-BR' };

const REPO = 'https://github.com/bps2414/feathertune';
export const links = {
  repo: REPO,
  latest: `${REPO}/releases/latest`,
  releases: `${REPO}/releases`,
  license: `${REPO}/blob/main/LICENSE`,
};

const en = {
  meta: {
    title: 'feathertune: your music, without the weight',
    description:
      'A light Windows app for the music website you already use. Native title bar, mini player, media keys, synced lyrics word by word, and about 3 MB to install.',
  },
  nav: { github: 'GitHub', language: 'Language' },
  suggest: { text: 'This page is also in Portuguese.', go: 'Ver em português', dismiss: 'Keep English' },
  hero: {
    sung: 'Your music, without the weight.',
    next: 'The music site you already use, in a light Windows app.',
    download: 'Download for Windows',
    fine: 'Windows 10 and 11. About 3 MB, installs without admin, updates itself.',
    source: 'Read the source on GitHub',
    replay: 'Sing it again',
  },
  weight: {
    title: 'Weighed, not guessed',
    lede: 'It uses the WebView2 runtime that already ships with Windows instead of bundling its own Chromium, and does the desktop parts in Rust.',
    stats: [
      { value: '3', unit: 'MB', label: 'to download and install' },
      { value: '2', unit: 's', label: 'from click to playing' },
      { value: '190', unit: 'MB', label: 'of memory while it sits in the tray' },
      { value: '4', unit: '%', label: 'of one CPU core while a song plays' },
    ],
    note: 'Measured on the maintainer’s PC, whole process tree, medians of several runs.',
    noteLink: 'How it was measured',
  },
  records: {
    title: 'It takes the color of the record',
    lede: 'The title bar, the mini player and the site take the colors of the cover, once per track. Ambient mode puts the cover, blurred and glowing, behind the page.',
    pick: 'Pick a record',
    by: 'by',
    mini: 'Mini player, always on top',
  },
  lyrics: {
    title: 'Lyrics that light up as they’re sung',
    lede: 'Time-synced lyrics from four open sources, in the site’s own Lyrics tab.',
    items: [
      {
        title: 'Word by word',
        text: 'Each word lights up as it is sung. When a source only times whole lines, the words are timed from the song’s own pace.',
      },
      { title: 'A translation under each line', text: 'Into your Windows language, from the source when it has one.' },
      { title: 'Floating lyrics', text: 'The current line over your desktop, and clicks pass right through it.' },
      { title: 'Lyric cards', text: 'Pick the lines you want, choose a look, then copy or save a picture.' },
    ],
    card: ['Your music,', 'without the weight.'],
  },
  never: {
    title: 'What it will never do',
    lede: 'It opens the official site, unmodified. These lines don’t move.',
    items: [
      { title: 'Block, skip or mute ads', text: 'Every audio effect steps aside while an ad plays.' },
      { title: 'See your password', text: 'You sign in on the site itself, exactly as in a browser.' },
      { title: 'Download or record audio', text: 'Only numbers like tempo and key are kept, on your PC.' },
      { title: 'Use private APIs', text: 'No stream extraction, no pretending to be another browser.' },
      { title: 'Send your history anywhere', text: 'Listening stats stay on this PC. Last.fm only if you connect it.' },
    ],
  },
  switches: {
    title: 'Every feature is a switch',
    lede: 'Everything starts on and can be turned off in the settings, which show what each one costs. Experimental ones start off.',
    on: 'On by default',
    off: 'Experimental, off by default',
    experimental: 'Experimental',
    items: [
      { title: 'Media keys and the Windows media flyout', on: true },
      { title: 'Tray icon that trims memory while hidden', on: true },
      { title: 'Discord status and Last.fm scrobbling', on: true },
      { title: 'Equalizer, compressor and visualizer', on: true },
      { title: 'Listening stats, kept on this PC', on: true },
      { title: 'Game mode: one click turns off the costliest features', on: true },
      { title: 'Crossfade and hand-made mixes', on: false },
      { title: 'Phone remote, end-to-end encrypted', on: false },
    ],
  },
  install: {
    title: 'Install it in a minute',
    steps: [
      { title: 'Download', text: 'Get the installer from the latest release.' },
      { title: 'Run it', text: 'It installs for your user only, no admin. New versions install themselves.' },
      { title: 'Sign in on the site', text: 'Inside the app, like in any browser. Your sign-in stays on your PC.' },
    ],
    remote: 'Android phone remote (experimental): the APK is attached to the releases.',
    releases: 'See all releases',
  },
  footer: {
    disclaimer:
      'An independent, unofficial project. It is not affiliated with, endorsed or sponsored by any music streaming service. All trademarks belong to their owners.',
    license: 'MIT license',
    made: 'Made with Rust and Tauri.',
  },
};

export type Strings = typeof en;

const ptBr: Strings = {
  meta: {
    title: 'feathertune: sua música, sem o peso',
    description:
      'Um app leve para Windows com o site de música que você já usa. Barra de título nativa, mini player, teclas de mídia, letras sincronizadas palavra por palavra e cerca de 3 MB para instalar.',
  },
  nav: { github: 'GitHub', language: 'Idioma' },
  suggest: { text: 'This page is also in Portuguese.', go: 'Ver em português', dismiss: 'Keep English' },
  hero: {
    sung: 'Sua música, sem o peso.',
    next: 'O site de música que você já usa, num app leve para Windows.',
    download: 'Baixar para Windows',
    fine: 'Windows 10 e 11. Cerca de 3 MB, instala sem admin e se atualiza sozinho.',
    source: 'Ver o código no GitHub',
    replay: 'Cantar de novo',
  },
  weight: {
    title: 'Pesado, não chutado',
    lede: 'Ele usa o runtime WebView2 que já vem no Windows em vez de carregar um Chromium próprio, e faz a parte de desktop em Rust.',
    stats: [
      { value: '3', unit: 'MB', label: 'para baixar e instalar' },
      { value: '2', unit: 's', label: 'do clique até tocar' },
      { value: '190', unit: 'MB', label: 'de memória enquanto fica na bandeja' },
      { value: '4', unit: '%', label: 'de um núcleo da CPU tocando uma música' },
    ],
    note: 'Medido no PC do mantenedor, árvore de processos inteira, mediana de várias execuções.',
    noteLink: 'Como foi medido',
  },
  records: {
    title: 'Ele pega a cor do disco',
    lede: 'A barra de título, o mini player e o site pegam as cores da capa, uma vez por faixa. O modo ambiente põe a capa, desfocada e brilhando, atrás da página.',
    pick: 'Escolha um disco',
    by: 'de',
    mini: 'Mini player, sempre por cima',
  },
  lyrics: {
    title: 'Letras que acendem enquanto são cantadas',
    lede: 'Letras sincronizadas de quatro fontes abertas, na própria aba Letra do site.',
    items: [
      {
        title: 'Palavra por palavra',
        text: 'Cada palavra acende quando é cantada. Quando a fonte só marca o tempo das linhas, as palavras seguem o ritmo da própria música.',
      },
      { title: 'Tradução embaixo de cada linha', text: 'No idioma do seu Windows, da própria fonte quando ela tem.' },
      { title: 'Letra flutuante', text: 'A linha atual por cima da área de trabalho, e os cliques passam direto por ela.' },
      { title: 'Cartões de letra', text: 'Escolha as linhas, escolha um visual, copie ou salve a imagem.' },
    ],
    card: ['Sua música,', 'sem o peso.'],
  },
  never: {
    title: 'O que ele nunca vai fazer',
    lede: 'Ele abre o site oficial, sem modificar. Estas linhas não mudam.',
    items: [
      { title: 'Bloquear, pular ou silenciar anúncios', text: 'Todo efeito de áudio sai do caminho enquanto um anúncio toca.' },
      { title: 'Ver sua senha', text: 'Você entra no próprio site, igual num navegador.' },
      { title: 'Baixar ou gravar áudio', text: 'Só números como tempo e tom ficam guardados, no seu PC.' },
      { title: 'Usar APIs privadas', text: 'Nada de extrair stream nem se passar por outro navegador.' },
      { title: 'Mandar seu histórico para algum lugar', text: 'As estatísticas ficam neste PC. Last.fm só se você conectar.' },
    ],
  },
  switches: {
    title: 'Cada recurso é um interruptor',
    lede: 'Tudo começa ligado e pode ser desligado nas configurações, que mostram quanto cada um custa. Os experimentais começam desligados.',
    on: 'Ligado por padrão',
    off: 'Experimental, desligado por padrão',
    experimental: 'Experimental',
    items: [
      { title: 'Teclas de mídia e o painel de mídia do Windows', on: true },
      { title: 'Ícone na bandeja que enxuga a memória quando escondido', on: true },
      { title: 'Status no Discord e scrobble no Last.fm', on: true },
      { title: 'Equalizador, compressor e visualizador', on: true },
      { title: 'Estatísticas de escuta, guardadas neste PC', on: true },
      { title: 'Modo jogo: um clique desliga os recursos mais pesados', on: true },
      { title: 'Crossfade e mixes feitas à mão', on: false },
      { title: 'Controle pelo celular, criptografado de ponta a ponta', on: false },
    ],
  },
  install: {
    title: 'Instale em um minuto',
    steps: [
      { title: 'Baixe', text: 'Pegue o instalador na versão mais recente.' },
      { title: 'Rode', text: 'Instala só para o seu usuário, sem admin. As versões novas se instalam sozinhas.' },
      { title: 'Entre no site', text: 'Dentro do app, como em qualquer navegador. Seu login fica no seu PC.' },
    ],
    remote: 'Controle pelo celular Android (experimental): o APK vem junto das versões.',
    releases: 'Ver todas as versões',
  },
  footer: {
    disclaimer:
      'Um projeto independente e não oficial. Não é afiliado, endossado nem patrocinado por nenhum serviço de streaming de música. Todas as marcas pertencem aos seus donos.',
    license: 'Licença MIT',
    made: 'Feito com Rust e Tauri.',
  },
};

export const strings: Record<Locale, Strings> = { en, 'pt-br': ptBr };

export const records = [
  {
    id: 'a',
    title: 'Lanterns on the Water',
    artist: 'Mira Holt',
    lines: ['we let the lanterns go', 'and watched the river carry them', 'past the bridge, past the light'],
  },
  {
    id: 'b',
    title: 'Low Tide Radio',
    artist: 'The Quiet Engines',
    lines: ['static on the low tide station', 'someone plays our song again', 'and the harbor hums along'],
  },
  {
    id: 'c',
    title: 'Honey in the Cracks',
    artist: 'Saul Ambrose',
    lines: ['sun gets in through every crack', 'honey on the windowsill', 'slow as Sunday, sweet and still'],
  },
] as const;
