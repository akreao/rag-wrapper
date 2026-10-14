# rAthena, built from source for one client packet version.
# Build args choose the source and the build; the configs are written at
# start-up by entrypoint.sh from environment variables.

FROM ubuntu:24.04 AS build

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential ca-certificates git \
      libmariadb-dev libmariadb-dev-compat libpcre3-dev zlib1g-dev \
 && rm -rf /var/lib/apt/lists/*

# RATHENA_COMMIT is a full commit SHA on the fork's rag-wrapper branch.
ARG RATHENA_REPO=https://github.com/akreao/rathena.git
ARG RATHENA_COMMIT=b6f89abe300d821412380a2004385532d0d4d9b6
RUN git init -q /src/rathena \
 && git -C /src/rathena fetch -q --depth 1 "${RATHENA_REPO}" "${RATHENA_COMMIT}" \
 && git -C /src/rathena checkout -q FETCH_HEAD

WORKDIR /src/rathena
ARG PACKETVER=20250716
ARG PRERE=yes
RUN ./configure --enable-packetver="${PACKETVER}" \
      $( [ "${PRERE}" = "yes" ] && echo "--enable-prere=yes" ) \
 && make server -j"$(nproc)"


FROM ubuntu:24.04

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      libmariadb3 libpcre3 zlib1g mariadb-client python3-yaml \
 && rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --home-dir /rathena rathena
WORKDIR /rathena

COPY --from=build /src/rathena/login-server /src/rathena/char-server \
                  /src/rathena/map-server /src/rathena/web-server ./
COPY --from=build /src/rathena/conf ./conf
COPY --from=build /src/rathena/db ./db
COPY --from=build /src/rathena/npc ./npc
COPY --from=build /src/rathena/sql-files ./sql-files
COPY entrypoint.sh merge_mods.py /usr/local/bin/

RUN mkdir -p log save \
 && chown -R rathena:rathena /rathena \
 && chmod +x /usr/local/bin/entrypoint.sh /usr/local/bin/merge_mods.py

USER rathena
ENTRYPOINT ["entrypoint.sh"]
