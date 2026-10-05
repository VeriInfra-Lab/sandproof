#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>

/* Dedicated ACL fixture, never added to an original benchmark template.
 * Build statically for SDK upload. Success prints exactly allowed/denied;
 * a broken probe exits 2 with a fixed diagnostic, never a policy-denied pass.
 * TCP proves a handshake; DNS proves a matching reply, not query-content ACLs.
 */
static int failure(const char *stage) {
    fprintf(stderr, "probe-error:%s\n", stage);
    return 2;
}

static int network_denial(int code) {
    return code == EACCES || code == EPERM || code == ENETDOWN ||
           code == ENETUNREACH || code == EHOSTUNREACH ||
           code == ECONNREFUSED || code == ECONNRESET || code == ETIMEDOUT;
}

static int tcp_probe(int fd, const struct sockaddr_in *peer, int *allowed) {
    int flags = fcntl(fd, F_GETFL);
    if (flags < 0 || fcntl(fd, F_SETFL, flags | O_NONBLOCK) < 0)
        return failure("fcntl");
    if (connect(fd, (const struct sockaddr *)peer, sizeof(*peer)) == 0) {
        *allowed = 1;
        return 0;
    }
    if (errno != EINPROGRESS)
        return network_denial(errno) ? 0 : failure("connect");
    struct pollfd event = {.fd = fd, .events = POLLOUT};
    int ready = poll(&event, 1, 3000);
    if (ready < 0) return failure("poll");
    if (ready == 0) return 0;
    if ((event.revents & POLLNVAL) ||
        !(event.revents & (POLLOUT | POLLERR | POLLHUP)))
        return failure("poll-events");
    int error = 0;
    socklen_t length = sizeof(error);
    if (getsockopt(fd, SOL_SOCKET, SO_ERROR, &error, &length) < 0 ||
        length != sizeof(error))
        return failure("getsockopt");
    if (error == 0) *allowed = 1;
    else if (!network_denial(error)) return failure("connect-result");
    return 0;
}

static int dns_probe(int fd, const struct sockaddr_in *peer, int *allowed) {
    unsigned char query[] = {0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0,
        6, 'g', 'i', 't', 'h', 'u', 'b', 3, 'c', 'o', 'm', 0, 0, 1, 0, 1};
    uint16_t id = (uint16_t)getpid();
    query[0] = (unsigned char)(id >> 8);
    query[1] = (unsigned char)id;
    for (int attempt = 0; attempt < 3; ++attempt) {
        ssize_t sent = sendto(fd, query, sizeof(query), 0,
                             (const struct sockaddr *)peer, sizeof(*peer));
        if (sent < 0) return network_denial(errno) ? 0 : failure("sendto");
        if (sent != (ssize_t)sizeof(query)) return failure("sendto-length");
        unsigned char response[4096];
        struct sockaddr_in from;
        socklen_t length = sizeof(from);
        ssize_t received = recvfrom(fd, response, sizeof(response), 0,
                                    (struct sockaddr *)&from, &length);
        if (received < 0) {
            if (errno == EAGAIN || errno == EWOULDBLOCK) continue;
            return network_denial(errno) ? 0 : failure("recvfrom");
        }
        if (length != sizeof(from) || from.sin_family != AF_INET ||
            from.sin_addr.s_addr != peer->sin_addr.s_addr ||
            from.sin_port != peer->sin_port || received < 12 ||
            response[0] != query[0] || response[1] != query[1] ||
            !(response[2] & 0x80) || (response[2] & 0x78))
            return failure("dns-response");
        /* Even NXDOMAIN/SERVFAIL proves a reply path. Treating it as denial
         * would wrongly pass a negative ACL test when the resolver is reachable.
         */
        *allowed = 1;
        return 0;
    }
    return 0;
}

int main(int argc, char **argv) {
    if (argc != 4 || (strcmp(argv[1], "dns") && strcmp(argv[1], "tcp")))
        return failure("arguments");
    if (!argv[3][0]) return failure("port");
    for (const char *digit = argv[3]; *digit; ++digit)
        if (*digit < '0' || *digit > '9') return failure("port");
    char *end;
    errno = 0;
    long port = strtol(argv[3], &end, 10);
    if (errno || *end || port < 1 || port > 65535) return failure("port");
    struct sockaddr_in peer = {.sin_family = AF_INET,
                              .sin_port = htons((uint16_t)port)};
    if (inet_pton(AF_INET, argv[2], &peer.sin_addr) != 1)
        return failure("address");
    int dns = !strcmp(argv[1], "dns");
    int fd = socket(AF_INET, dns ? SOCK_DGRAM : SOCK_STREAM, 0);
    if (fd < 0) return failure("socket");
    struct timeval timeout = {.tv_sec = 3};
    if (setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout)) < 0 ||
        setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout)) < 0) {
        (void)close(fd);
        return failure("setsockopt");
    }
    int allowed = 0;
    int result = dns ? dns_probe(fd, &peer, &allowed) : tcp_probe(fd, &peer, &allowed);
    if (close(fd) < 0 && result == 0) return failure("close");
    if (result) return result;
    puts(allowed ? "allowed" : "denied");
    return 0;
}
