from mitmproxy.certs import CertStore
from mitmproxy import ctx
from pathlib import Path

CertStore.create_store(Path.home() / ".mitmproxy", "mitmproxy", 2048)
ctx.master.shutdown()
