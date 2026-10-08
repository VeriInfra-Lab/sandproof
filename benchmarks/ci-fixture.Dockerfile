# One-time CI fixture preparation, not an original benchmark environment.
# BASE_IMAGE must be the reviewed task's immutable Linux/amd64 OCI digest.
ARG BASE_IMAGE
FROM ${BASE_IMAGE}
USER root
COPY ci-uv /usr/local/bin/ci-uv
COPY verifier-requirements.txt /opt/sandproof-ci-build/verifier-requirements.txt
ENV UV_PYTHON_INSTALL_DIR=/opt/sandproof-ci-python
ENV PATH=/opt/sandproof-ci-verifier/bin:${PATH}
RUN /usr/local/bin/ci-uv python install 3.13.7 \
    && /usr/local/bin/ci-uv venv --python 3.13.7 /opt/sandproof-ci-verifier \
    && /usr/local/bin/ci-uv pip install --python /opt/sandproof-ci-verifier/bin/python \
        --index-url https://pypi.org/simple \
        -r /opt/sandproof-ci-build/verifier-requirements.txt \
    && /opt/sandproof-ci-verifier/bin/python -I -c \
        'from importlib.metadata import version; import sys; assert sys.version_info[:3] == (3,13,7); assert version("pytest") == "8.4.1"; assert version("pytest-json-ctrf") == "0.3.5"' \
    && rm /usr/local/bin/ci-uv
