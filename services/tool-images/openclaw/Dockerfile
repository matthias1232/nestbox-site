FROM node:24-bookworm-slim
RUN npm install --global openclaw@latest
CMD ["openclaw", "gateway", "--listen", "0.0.0.0:8080"]

# nestbox-sandbox-user: the sandbox does not run as root.
#
# A gateway (OpenShell) sandbox refuses to start when its working directory
# belongs to another user than the sandbox identity, and an agent that signs in
# on a root account has the whole container to damage. The user is added here,
# after every install step, so the image layers above stay cached.
# The steps below need root to create the account, whatever the base image left set.
USER root
RUN id -u nestbox >/dev/null 2>&1 || useradd -m -d /home/nestbox -u 10001 -s /bin/bash nestbox \
 && install -d -o nestbox -g nestbox /workspace /home/nestbox \
 && chown -R nestbox /workspace /home/nestbox
USER nestbox
WORKDIR /workspace
