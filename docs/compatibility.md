# Compatibilidade do motor

O port preserva o algoritmo de canonicalização do Tailwind CSS 4.3.4, incluindo utilitários, variantes, assinaturas, combinação de classes, configuração CSS e ordenação. As descrições de utilitários em `native/data/` são metadados de implementação, não uma tabela de respostas para entradas do app.

As fixtures foram produzidas durante a validação do port contra a fonte oficial vendorizada. Elas cobrem casos do upstream, entradas sintéticas, fuzz e strings de classes do projeto de origem. Os arquivos completos desse app não fazem parte deste repositório. Os testes de stylesheet incluídos aqui usam somente os quatro stylesheets oficiais do Tailwind.

## Expressões regulares

O Tailwind constrói algumas expressões a partir de nomes de variáveis CSS sem escapar seu conteúdo. A compatibilidade exige a semântica ECMAScript Annex B e detalhes do V8 interpretado, inclusive casos que diferem da interpretação intuitiva de grupos de modificadores de flags.

O backend é Rust seguro, com um grafo próprio e uma versão modificada do crate `regress`. Os arquivos de origem, licença, patches e seus manifests estão em `native/vendor/`. As fixtures de regex são estritas; o arquivo de diferenças históricas registra a investigação anterior e não dispensa resultados divergentes nos testes atuais.

Essa referência fixa não acompanha mudanças de tier de compilação do V8. Também não emula a pressão acumulada de memória executável de um processo Node. Representação one-byte/two-byte e limites de recursos são modelados conforme os dados de referência. Memória, tamanho e pilha disponíveis em outra máquina podem mudar o ponto de falha de padrões extremos.

Backtracking pode ter custo exponencial em expressões adversariais. Não há garantia de tempo linear nem sandbox para processar repositórios não confiáveis. Os testes preservam as regressões conhecidas, mas não estabelecem equivalência universal entre os motores.

## Atualização do Tailwind

Alterar a versão exige revisar código e metadados, gerar novas fixtures contra a versão fixada e validar novamente o port. Instalar uma versão diferente de `tailwindcss` no consumidor não atualiza automaticamente a implementação embutida.

O suporte a configuração é CSS. Configs e plugins JavaScript não são executados. O escopo não inclui gerar uma folha Tailwind completa para produção, source maps ou funcionar como substituto da CLI oficial de build.
