# twcanon

CLI nativa para canonicalizar classes do Tailwind CSS 4.3.4 em projetos JS/TS e React. Converte grafias equivalentes, combina utilitários compatíveis e ordena classes de acordo com o tema CSS do projeto.

```tsx
cn("flex-grow w-4 h-4");
// após twcanon:
cn("size-4 grow");
```

O motor e o parser de código são Rust. JavaScript participa da instalação via npm; a execução da CLI usa diretamente o binário, sem Node, N-API ou subprocesso JavaScript. O código próprio proíbe `unsafe`.

## Instalação

```sh
npm install --save-dev twcanon
# ou
pnpm add -D twcanon
# ou
bun add -d twcanon
```

Permita dependências opcionais e o script de instalação. Se o gerenciador bloquear scripts, execute:

```sh
node node_modules/twcanon/install.mjs
```

Node 22 ou superior é necessário para instalar. O pacote escolhe o binário da plataforma e verifica versão, arquitetura e SHA-256 antes de ativá-lo. Essa verificação detecta corrupção; depende da integridade dos próprios pacotes npm.

## Uso

Execute na raiz do projeto:

```sh
twcanon --check                 # verifica src/, sem escrever
twcanon                         # corrige src/
twcanon --check src/components   # limita aos caminhos informados
twcanon --css src/theme.css      # CSS de entrada alternativo
twcanon --no-cache --check       # ignora cache persistente
```

O CSS padrão é `src/index.css`. Os caminhos são arquivos ou diretórios dentro de `src/`; globs devem ser expandidos pelo shell. Alvos inexistentes não produzem alterações. `--serial` é aceito por compatibilidade. Use `--` antes de um caminho que comece com `-`.

```json
{
  "scripts": {
    "lint:classes": "twcanon --check",
    "fix:classes": "twcanon"
  }
}
```

| Saída | Significado |
| --- | --- |
| `0` | Nenhuma diferença, ou correções concluídas |
| `1` | `--check` encontrou classes a corrigir |
| `2` | Erro de configuração, sintaxe ou I/O |

## Contrato

- Examina `.js`, `.jsx`, `.ts` e `.tsx` em `src/`. Ignora `.d.ts`, `.gen.ts`, links simbólicos encontrados durante a varredura e os arquivos gerados `src/lib/tailwind-merge.ts`, `src/lib/tailwind-merge-data.ts` e `src/types/icon.ts`.
- Extrai strings e templates sem interpolação de `class`/`className` em JSX e dos argumentos literais de chamadas diretas a `cn`, `twMerge` e `mergeClassName`. Expressões dinâmicas, chaves de objetos e chamadas por propriedade ficam intactas.
- Carrega imports CSS locais e de pacotes, `@theme`, prefixos, `@utility`, `@custom-variant` e `@apply`. O projeto deve instalar o Tailwind e os pacotes CSS que seu stylesheet importar. `@config` e `@plugin` que executariam JavaScript produzem erro explícito.
- Usa `rem = 16`, combinação de utilitários e conversão de propriedades lógicas para físicas, conforme o fluxo para o qual foi desenvolvido. Avalie essa conversão se o projeto depender de escrita RTL ou vertical.
- Mantém cache em `.cache/twcanon.json`, invalidado pelo executável, conteúdo do CSS e resolução dos imports. `CI` ou `--no-cache` desativam leitura e escrita desse cache. É seguro remover o diretório; valores acumulados não têm poda automática.
- Planeja as alterações e valida sintaxe e acesso de escrita antes de gravar. Cada arquivo é substituído por rename atômico. O lote inteiro não é atômico: uma falha de I/O informa quais arquivos já foram alterados.
- Recusa correção em arquivos somente leitura e caminhos com symlink. A substituição não preserva identidade de hardlinks, proprietário ou atributos estendidos. Existe uma janela entre a checagem e o rename; evite editar os mesmos arquivos simultaneamente.
- Recusa texto não UTF-8 e surrogate UTF-16 isolado com indicação de arquivo e linha. Não substitui dados inválidos silenciosamente.

## Compatibilidade

O comportamento de referência é Tailwind CSS **4.3.4** e, para expressões regulares, Node **24.21.0 / V8 13.6.233.17-node.53**, com `--regexp-interpret-all` e a representação de strings fixada nas fixtures. Isso não implica suporte automático a outras versões do Tailwind.

A suíte diferencial preserva os resultados da referência. Casos amostrados não provam equivalência geral. O contrato do motor de regex e seus limites estão em [docs/compatibility.md](docs/compatibility.md).

Há pacotes para Linux x64/arm64 (glibc e musl), macOS x64/arm64 e Windows x64/arm64. As releases só são publicadas após build e teste de instalação de todos os alvos. Linux glibc requer 2.39 ou superior nos artefatos gerados pelo CI; os binários musl são treinados e testados também em Alpine. A versão mínima de macOS usada para compilação é 13.0.

## Desenvolvimento

Requisitos: Rust **1.99.1**, Node **22+**, Python **3** e ferramentas de link da plataforma. O toolchain está fixado em `native/rust-toolchain.toml`. Não há dependências JS de desenvolvimento para instalar.

```sh
node scripts/fixtures.mjs        # confere hashes e extrai fixtures comprimidas
node scripts/check.mjs           # fmt, testes Rust, Clippy, instalador e CLI
node scripts/build.mjs           # release convencional
node scripts/build.mjs --pgo     # release com treinamento PGO sintético
node scripts/pack.mjs            # dois tarballs locais em artifacts/
node scripts/test-package.mjs artifacts
```

PGO exige o componente `llvm-tools-preview` do toolchain. O script gera perfis novos para o checkout e o alvo, treina sem cache e registra hash do binário e do perfil em `build/build.json`. Não reutiliza perfis dependentes de caminhos de outra máquina. Os builds de release do CI usam PGO; não há promessa de ganho fixo de desempenho para qualquer projeto.

Para testar um build PGO na suíte CLI, informe `TWCANON_BINARY` com o caminho absoluto do executável. A biblioteca tem nome interno `tailwind_canonical_native`; sua distribuição por Cargo está desativada. A interface publicada é a CLI.

## Publicação

O fluxo está em [docs/releasing.md](docs/releasing.md). O CI monta oito pacotes nativos e o pacote principal, testa instalação em cada plataforma e publica os nativos antes do principal. Publicação usa Trusted Publishing do npm, sem token armazenado no repositório.

Código sob MIT. Os componentes derivados de Tailwind, V8, Unicode e as dependências Rust conservam seus avisos em `licenses/` e `native/vendor/`.
