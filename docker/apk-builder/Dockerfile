FROM node:20-bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive \
    ANDROID_HOME=/opt/android-sdk \
    JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64 \
    PATH=/opt/android-sdk/cmdline-tools/latest/bin:/opt/android-sdk/platform-tools:/opt/android-sdk/build-tools/34.0.0:/usr/lib/jvm/java-17-openjdk-amd64/bin:$PATH

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates curl unzip git openjdk-17-jdk-headless \
 && rm -rf /var/lib/apt/lists/*

ARG ANDROID_TOOLS_VERSION=11076708
RUN mkdir -p ${ANDROID_HOME}/cmdline-tools \
 && curl -fsSL -o /tmp/tools.zip \
        "https://dl.google.com/android/repository/commandlinetools-linux-${ANDROID_TOOLS_VERSION}_latest.zip" \
 && unzip -q /tmp/tools.zip -d ${ANDROID_HOME}/cmdline-tools \
 && mv ${ANDROID_HOME}/cmdline-tools/cmdline-tools ${ANDROID_HOME}/cmdline-tools/latest \
 && rm /tmp/tools.zip \
 && yes | sdkmanager --licenses >/dev/null \
 && sdkmanager --install \
        "platform-tools" \
        "platforms;android-34" \
        "build-tools;34.0.0" >/dev/null \
 && ln -s ${ANDROID_HOME}/cmdline-tools/latest ${ANDROID_HOME}/tools

WORKDIR /builder
COPY package.json ./
RUN npm install --omit=dev

COPY build.js ./

ENV OUTPUT_DIR=/data/apk \
    KEYSTORE_DIR=/keystore \
    WORK_DIR=/work

ENTRYPOINT ["node", "/builder/build.js"]
