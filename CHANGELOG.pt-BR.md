# Changelog

O que mudou em cada versão do feathertune. Os instaladores estão na [página de releases](https://github.com/bps2414/feathertune/releases). (English: [CHANGELOG.md](CHANGELOG.md))

## 0.1.7 — 2026-10-20

- **Playlists longas não somem mais.** Com o modo ambiente ligado, a lista de faixas de uma playlist ou álbum longo desaparecia poucas linhas abaixo. Agora rola normalmente.

## 0.1.6 — 2026-10-20

### Mini player
- **A letra rola suave.** Na troca de linha, a próxima sobe para o lugar e cresce, a antiga some para cima e a nova próxima aparece de baixo. As linhas não ficam mais uma por cima da outra.
- **Botões maiores e mais claros.** Play/pausa é um disco grande, anterior e próxima têm área maior, e os botões de cima (letra, janela completa, fechar) ficam sobre pílulas suaves e dizem o que fazem ao passar o mouse.

### Compartilhar letra
- **Cartão mais bonito.** A cor principal do álbum pinta as aspas, uma barra ao lado das linhas e os pontinhos de pulo. As linhas ganham uma sombra suave, a capa uma borda clara, Ink e Dark uma vinheta, e todo visual um grão de filme fino.
- **Uma nota musical de verdade** no rodapé, desenhada sobre um disco tingido, no lugar do glifo de fonte antigo.

## 0.1.5 — 2026-10-20

- **Botão de compartilhar na barra de título**, que só aparece quando a faixa tem letra sincronizada.
- **Cartão de compartilhar mais bonito:** aspas grandes, brilho suave, borda de vidro, capa maior e um rodapé com o álbum. Sem nome do app no cartão.
- **Letra no mini player:** um botão novo adiciona uma faixa com a linha atual e a próxima.
- **Correção no menu da bandeja:** ele não fica mais aberto depois de um clique em outro lugar.

## 0.1.4 — 2026-10-20

- **Compartilhar letra:** escolha quaisquer linhas (até 8, Shift-clique para um intervalo) e gere um cartão em quatro visuais: Cover, Ink, Dark e Paper. Copie para a área de transferência ou salve como PNG de 1080 px.
- **Letra flutuante:** a linha atual e a próxima flutuam sobre a área de trabalho, deixando o clique passar quando travada.
- **Controles na barra de tarefas:** anterior, play/pausa e próxima na miniatura, e o progresso da faixa no botão.
- **Menu da bandeja** nas cores do álbum.
- **Estatísticas de escuta:** histórico local com uma aba Stats nas configurações e exportação em CSV.
- **A rolagem da letra** estaciona na linha atual e passa para a próxima em volta da troca, então uma linha que está acabando não fica lá em cima antes da hora.

## 0.1.3 — 2026-10-20

- **Letra:** as linhas fazem transição cruzada e, com a música tocando, a lista rola no ritmo dela, suavizando depois de seeks, cliques e rolagem manual.
- **Animações** seguem os próprios interruptores nas configurações, não os efeitos de animação do Windows.
- **Modo ambiente** ligado por padrão, como todo plugin.
- **Correção:** o fundo do site não se repete mais como uma linha a cada meia tela no modo ambiente.

## 0.1.2 — 2026-10-20

- **Tela de início** que mostra o que o app está fazendo enquanto procura atualizações e pré-carrega o site.
- **Atualização automática** pela última release: baixada, conferida pelo SHA-256 e instalada em silêncio, e o app reinicia. Só cópias instaladas se atualizam.
- **Letra:** texto maior, linhas esmaecem conforme a distância da atual, clique numa linha para pular até ela.
- **Correção:** a barra "Reiniciar agora" das configurações aparecia sempre.

## 0.1.1 — 2026-10-20

- **Correção: scrobble no Last.fm.** O scrobbler parava no primeiro prazo, então nada era registrado.
- **Letra:** texto maior, e entradas com tempo por palavra acendem palavra por palavra.
- **Discord:** presença mais rica, com a capa da faixa, links e um botão.
- **Modo ambiente:** o fundo de cima esmaece direito; caixa de busca sólida.

## 0.1.0 — 2026-10-20

Primeira versão.

- O player web oficial do site numa janela WebView2 enxuta, com teclas de mídia e ícone na bandeja.
- **Barra de título nativa** com visual de vidro líquido nas cores do álbum, e um **mini player nativo**, sempre no topo.
- **Janela de configurações** com um interruptor para cada plugin e campos de atalho.
- **Cores do álbum** na barra de título, no mini player e na página, e um **modo ambiente** opcional.
- **Letra sincronizada** do LRCLIB na aba Letra do site.
- Scrobble no **Last.fm** (com fila offline) e **Rich Presence no Discord**.
- **Filtro de privacidade** contra rastreadores (só listas de privacidade; anúncios e reprodução nunca são tocados).
- **Instalador por usuário**, que também pode apagar seus dados na desinstalação se você marcar a caixa.
