import json
import sys
import nbformat
from nbclient import NotebookClient
from jupyter_client import KernelManager
from jupyter_client.kernelspec import KernelSpecManager
from test_bindings import ROOT


def test_notebook_gallery_executes_with_the_test_interpreter(tmp_path):
    spec = tmp_path / "pelagic-test"
    spec.mkdir()
    (spec / "kernel.json").write_text(json.dumps({
        "argv": [sys.executable, "-m", "ipykernel_launcher", "-f", "{connection_file}"],
        "display_name": "Pelagic test", "language": "python",
    }))
    notebooks = list((ROOT / "examples/notebooks").glob("*.ipynb"))
    assert notebooks
    for path in notebooks:
        manager = KernelManager(kernel_name="pelagic-test", kernel_spec_manager=KernelSpecManager(kernel_dirs=[str(tmp_path)]))
        try:
            NotebookClient(nbformat.read(path, as_version=4), km=manager, timeout=120,
                           resources={"metadata": {"path": str(ROOT)}}).execute()
        finally:
            if manager.has_kernel:
                manager.shutdown_kernel(now=True)
