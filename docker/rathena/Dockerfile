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
ARG RATHENA_COMMIT=266a587f6a0901bbd9d6935f359c7ff0bf65b9aa
RUN git init -q /src/rathena \
 && git -C /src/rathena fetch -q --depth 1 "${RATHENA_REPO}" "${RATHENA_COMMIT}" \
 && git -C /src/rathena checkout -q FETCH_HEAD

WORKDIR /src/rathena
ARG PACKETVER=20250716
ARG PRERE=yes
# g++ takes 1 to 2 GB for each of rAthena's largest files, so a machine
# with many cores and little memory runs out with one job per core. Empty
# MAKE_JOBS runs one job per 2 GB of available memory, at least 1 and at
# most one per core.
ARG MAKE_JOBS=
RUN ./configure --enable-packetver="${PACKETVER}" \
      $( [ "${PRERE}" = "yes" ] && echo "--enable-prere=yes" ) \
 && jobs="${MAKE_JOBS:-$(awk -v cores="$(nproc)" '/^MemAvailable:/ { j = int($2 / 2097152); if (j < 1) j = 1; if (j > cores) j = cores; print j }' /proc/meminfo)}" \
 && echo "Building with ${jobs:=1} jobs" \
 && make server -j"${jobs}"


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
# The stack's own NPC scripts, such as the desktop app's in-game
# announcements; entrypoint.sh loads every one after rAthena's own.
COPY npc/ npc/wrapper/

# The game mode is compiled in, so entrypoint.sh compares it with PRERE at
# start-up and stops when they differ.
ARG PRERE=yes
RUN mkdir -p log save \
 && echo "${PRERE}" > BUILD_PRERE \
 && chown -R rathena:rathena /rathena \
 && chmod +x /usr/local/bin/entrypoint.sh /usr/local/bin/merge_mods.py

USER rathena
ENTRYPOINT ["entrypoint.sh"]
