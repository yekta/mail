# The marketing site (apps/marketing), as static files behind static-web-server. Build from the
# repository root: docker build -f apps/marketing/Dockerfile .

FROM node:24-bookworm-slim AS build
RUN npm install -g pnpm@12.9.1
WORKDIR /app/apps/marketing
COPY apps/marketing/package.json apps/marketing/pnpm-lock.yaml apps/marketing/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile
COPY docs/screenshots /app/docs/screenshots
COPY apps/marketing ./
RUN pnpm build

FROM joseluisq/static-web-server:2.44.0
LABEL org.opencontainers.image.source=https://github.com/yekta/wonnet
COPY --from=build /app/apps/marketing/dist /public
COPY apps/marketing/server.toml /server.toml
ENV SERVER_CONFIG_FILE=/server.toml
EXPOSE 3000
