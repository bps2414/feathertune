# Changelog

O que mudou em cada versão do feathertune. Os instaladores estão na [página de releases](https://github.com/bps2414/feathertune/releases). (English: [CHANGELOG.md](CHANGELOG.md))

## 0.2.7 — 2026-10-22

- **Letras sobrepostas em músicas rápidas.** Quando uma segunda voz ou um eco começa antes de a linha anterior terminar, as duas linhas agora ficam acesas e acendem as próprias palavras até cada uma acabar, em vez de a primeira ser cortada no começo da próxima. Cada linha guarda o fim que a fonte informa.
- **Vozes de fundo.** Letras com vozes de fundo mostram essas vozes numa linha menor embaixo da principal, acesa palavra por palavra, em vez de deixá-las de fora.
- **Aba Letra mais suave.** As linhas agora esmaecem e crescem sem ser repintadas, e o brilho (mesmo visual, mesma cor do álbum) é uma camada própria, então trocar de linha ou acender uma palavra não repinta mais um desfoque. Letras guardadas antes desta versão são buscadas de novo uma vez para pegar o fim das linhas.

## 0.2.6 — 2026-10-21

- **A letra sincronizada não some mais depois de trocar de faixa.** Numa troca automática de faixa ou depois de um anúncio a letra era buscada com a duração errada, então nenhuma fonte batia e a faixa ficava sem letra sincronizada até ser tocada de novo. Agora a busca espera os dados da faixa assentarem, busca de novo se a duração mudar, e uma busca lenta não segura mais a próxima faixa.
- **Traduções chegam antes.** Não esperam mais a fonte de letra mais lenta.
- **Aba Letra mais suave.** Acabou o engasgo quando a próxima linha acende e o scroll estaciona, e o scroll desliza sem degraus de 1 pixel.
- **Modo jogo.** Um botão de controle na barra de título desliga na hora os recursos medidos como os mais caros (letra sincronizada na página, modo ambiente, janelas de letra flutuante e de compartilhar); um segundo clique traz tudo de volta.
- **Exportar diagnóstico** (Configurações > Advanced): um arquivo de texto com o que o app fez, para mandar quando algo der errado.

## 0.2.5 — 2026-10-21

- **Letra palavra por palavra em músicas rápidas.** As palavras acendem na hora em que são cantadas, também com a janela fora de foco, e palavras rápidas acendem de uma vez em vez de surgirem atrás da voz. A aba Letra rola num ritmo constante pelas linhas rápidas em vez de parar a cada uma, e não fica mais atrás da linha atual.

## 0.2.4 — 2026-10-21

- **O app em português.** Menus, mini player, letra flutuante e compartilhada, a tela de início, as configurações e as Novidades seguem o idioma do Windows; escolha English ou Português na aba Geral das configurações (vale depois de reiniciar).

## 0.2.3 — 2026-10-21

- **Tradução da letra.** Uma tradução embaixo de cada linha sincronizada, no idioma do Windows por padrão (ou num escolhido na aba Letras das configurações): a tradução da própria fonte quando ela tem, senão uma tradução automática feita para letras. O original continua acendendo palavra por palavra; a tradução acende com a linha. A letra flutuante e o mini player a mostram no lugar da próxima linha. Linhas que já estão nesse idioma ficam sem tradução, e dá para desligar.

## 0.2.2 — 2026-10-21

- **Discord Rich Presence funciona sem configurar nada.** O release traz o próprio id de aplicativo do Discord; um id digitado nas configurações continua tendo prioridade.

## 0.2.1 — 2026-10-21

- **A janela abre depois da tela de abertura, maximizada.** Ela podia ficar na bandeja, ou aparecer no tamanho normal quando estava maximizada.

## 0.2.0 — 2026-10-21

**Versão experimental.** Os recursos de áudio novos (crossfade, mixes, pular silêncio, análise de faixa) ainda estão crus e podem falhar, então vêm desligados, na nova aba Experimental das configurações. O resto vem ligado como sempre.

### Letras
- **Palavra por palavra com mais frequência.** Mais duas fontes abertas com tempo por palavra, todas consultadas ao mesmo tempo: a primeira resposta aparece na hora e uma com tempo por palavra a substitui quando chega.
- **Tempo estimado por palavra** para letras sincronizadas só por linha, a partir das sílabas e do ritmo da própria música (dá para desligar), mais um ajuste de atraso.
- **Uma aba Lyrics** nas configurações para escolher e ordenar as fontes.

### Áudio
- **Equalizador** (10 bandas, presets) e **compressor** (suave, noturno, forte), só neste PC. Todo efeito fica de fora enquanto toca um anúncio.
- **Visualizador:** barras ou uma onda embaixo da capa no player expandido.
- **Experimental:** crossfade com curvas, fade de entrada e eco de saída; pular silêncio no começo e no fim da faixa; análise local da faixa (tempo, tom, volume); mixes por playlist e por par de faixas, com transições sobrepostas quando a conta deixa.

### Player
- Botões de **voltar e avançar** na barra de título.
- **Modo música:** escolhe a versão música de uma faixa em vez do vídeo, então nenhum vídeo é decodificado.
- **Volume preciso:** a roda do mouse sobre o player muda o volume de 2% em 2% (10% com Shift), com teclas globais opcionais.
- **Pular faixas com dislike** que já começam marcadas.

### Janela e configurações
- **Tela cheia** pelo player ocupa a janela e sai sem travar; a página se reajusta depois de mudar de monitor ou de DPI. "Reload page" na bandeja.
- **As abas das configurações viraram uma barra lateral**, então todas ficam à vista, mais uma busca em todas elas.
- **What's new:** depois de uma atualização as configurações abrem nesta lista.

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
