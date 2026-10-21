# Spike S0-12 sandbox image: OpenShell base + polaris-guest + Xvfb + Chrome for Testing.
FROM nvcr.io/nvidia/base/ubuntu:24.04

ARG CFT_VERSION
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl unzip xvfb xauth x11-apps \
      fonts-liberation libnss3 libatk1.0-0t64 libatk-bridge2.0-0t64 libcups2t64 libdrm2 libxkbcommon0 \
      libxcomposite1 libxdamage1 libxfixes3 libxrandr2 libgbm1 libpango-1.0-0 libcairo2 libasound2t64 \
      libgtk-3-0t64 libxshmfence1 \
 && rm -rf /var/lib/apt/lists/*
# Under /usr: the default OpenShell sandbox policy does not allow executing from /opt (S0-12 run 1: exit 126).
RUN curl -fsSL "https://storage.googleapis.com/chrome-for-testing-public/${CFT_VERSION}/linux64/chrome-linux64.zip" -o /tmp/chrome.zip \
 && unzip -q /tmp/chrome.zip -d /usr/lib \
 && rm /tmp/chrome.zip \
 && ln -s /usr/lib/chrome-linux64/chrome /usr/local/bin/chrome

COPY polaris-guest /usr/local/bin/polaris-guest
COPY test.html /opt/polaris/test.html
