FROM python:3.12-slim
# The OpenShell CLI, pinned and verified. The runner drives the OpenShell gateway
# through it; the gateway itself runs on the host (deploy/openshell).
ARG OPENSHELL_VERSION=0.1.6
ARG OPENSHELL_SHA256=2951c67366f3e3584ed9cec8d49b9f0b0eefc8cacea8b0582ac164590492f136
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates restic \
 && curl -fLsS -o /tmp/openshell.tgz \
      "https://github.com/NVIDIA/OpenShell/releases/download/v${OPENSHELL_VERSION}/openshell-x86_64-unknown-linux-musl.tar.gz" \
 && echo "${OPENSHELL_SHA256}  /tmp/openshell.tgz" | sha256sum -c - \
 && tar -xzf /tmp/openshell.tgz -C /usr/local/bin openshell \
 && rm -rf /tmp/openshell.tgz /var/lib/apt/lists/* \
 && openshell --version
WORKDIR /app
COPY requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt
COPY app ./app
EXPOSE 9000
CMD ["uvicorn", "app.main:app", "--host", "0.0.0.0", "--port", "9000"]
