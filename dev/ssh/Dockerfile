FROM alpine:3.20
RUN apk add --no-cache openssh \
 && ssh-keygen -A \
 && adduser -D rgrip && echo "rgrip:rgrip" | chpasswd \
 && mkdir -p /home/rgrip/.ssh \
 && sed -i -E "/^#?(AllowTcpForwarding|PasswordAuthentication|PermitOpen)/d" /etc/ssh/sshd_config \
 && printf "PasswordAuthentication yes\nAllowTcpForwarding yes\nPermitOpen any\n" >> /etc/ssh/sshd_config
COPY id_ed25519.pub /home/rgrip/.ssh/authorized_keys
RUN chown -R rgrip:rgrip /home/rgrip/.ssh && chmod 700 /home/rgrip/.ssh && chmod 600 /home/rgrip/.ssh/authorized_keys
EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
