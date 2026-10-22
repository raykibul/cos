# Node toolchain for the desktop UI (`make ui-*`). pnpm comes through
# corepack, at the version package.json's `packageManager` names.
FROM node:22-bookworm
ENV COREPACK_ENABLE_DOWNLOAD_PROMPT=0 \
    npm_config_store_dir=/pnpm-store \
    CI=true
RUN corepack enable
