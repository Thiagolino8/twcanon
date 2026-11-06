# Publicação

## Preparação inicial

1. Crie o repositório público `Thiagolino8/twcanon` e envie o conteúdo para `main`.
2. Reserve e configure no npm o pacote `twcanon` e os oito nomes em `package.json`. Configure Trusted Publisher para cada pacote: proprietário `Thiagolino8`, repositório `twcanon`, workflow `release.yml`.
3. A primeira publicação pode exigir o procedimento de bootstrap indicado pelo npm para nomes ainda inexistentes. Faça isso na conta mantenedora; o workflow não contém credenciais e depende do Trusted Publisher já configurado.
4. Proteja `main`, exija o check `release-check` e restrinja quem pode criar tags de versão.

A criação deste repositório local não reserva nomes nem publica pacotes.

## Nova versão

Atualize a versão raiz e todas as `optionalDependencies`, além de `native/Cargo.toml` e da entrada correspondente em `native/Cargo.lock`. Preserve versões idênticas entre o pacote principal e os nativos.

Os pacotes Windows se chamam `twcanon-windows-x64` e `twcanon-windows-arm64`. O campo `os` permanece `win32`, como exige o npm; os alvos Rust continuam usando MSVC.

Execute `node scripts/check.mjs`. Envie a mudança e aguarde o CI. A tag `v<versão>`, apontando para a mudança validada, aciona o mesmo workflow com permissão OIDC de publicação.

O CI compila os oito alvos com Rust fixado e PGO sintético, testa os tarballs, valida formato e hash dos executáveis e só então publica. O pacote principal sai por último. Uma versão npm é imutável: a retomada aceita um pacote já publicado somente se seu `dist.integrity` for igual ao tarball validado; caso contrário, aborta.

Se a publicação falhar parcialmente, use **Re-run failed jobs** para reutilizar os artefatos já validados. Refazer todo o workflow recompila o PGO e pode gerar tarballs diferentes; nesse caso, a trava de integridade impede substituir uma versão já publicada.

## Validação local de artefatos

```sh
node scripts/build.mjs --pgo
node scripts/pack.mjs
node scripts/test-package.mjs artifacts
```

Isso produz apenas os pacotes do host e principal. Para validar o conjunto baixado do CI:

```sh
node scripts/release.mjs artifacts
```

O comando exige os nove tarballs da mesma versão. `--publish` só é aceito dentro do GitHub Actions, no repositório e na tag previstos. Nenhum comando de build ou teste local publica no npm.

## Licenças e fixtures

`licenses/index.json` registra os avisos e hashes das dependências fixadas no `Cargo.lock`, incluindo Rust stdlib. Mudanças de dependências exigem atualizar e conferir esses avisos antes de publicar. `node scripts/licenses.mjs` verifica o inventário atual.

Os fixtures são versionados como gzip determinístico; `native/fixtures/manifest.json` vincula conteúdo comprimido e extraído por SHA-256. Após uma atualização deliberada das fixtures, regenere os gzip e esse manifest; não atualize os hashes para contornar uma falha de equivalência.
