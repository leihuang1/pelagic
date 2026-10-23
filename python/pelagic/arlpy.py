"""Opt-in arlpy 1.9 backend; no external solver executable is used.

The bridge reuses arlpy's legacy serializer/readers in private temporary files,
then uses Pelagic's normal Rust validation and in-process solve. The core Python
API does not require these files or arlpy. Registration is explicit because
arlpy currently exposes only a private model registry.
"""
from pathlib import Path
from tempfile import TemporaryDirectory
from . import bellhop

_TASKS = {"arrivals": "A", "eigenrays": "E", "rays": "R", "coherent": "C",
          "incoherent": "I", "semicoherent": "S"}


class _Pelagic:
    def supports(self, env=None, task=None):
        return env is None or (env.get("type") == "2D" and task in _TASKS)

    def run(self, env, task, debug=False, fname_base=None):
        import arlpy.uwapm as pm
        if fname_base is not None:
            raise ValueError("Pelagic's arlpy bridge does not support fname_base; use Result.write_native()")
        with TemporaryDirectory(prefix="pelagic-arlpy-") as directory:
            adapter = pm._Bellhop()
            stem = adapter._create_env_file(env, _TASKS[task], str(Path(directory) / "input"))
            result = bellhop.load(stem + ".env").run()
            output = Path(directory) / "result"
            result.write_native(output)
            stem = str(output / "case")
            if debug:
                print(f"Pelagic {result.provenance.version}; case_sha256={result.provenance.case_sha256}")
            if task == "arrivals":
                return adapter._load_arrivals(stem)
            if task in ("rays", "eigenrays"):
                return adapter._load_rays(stem)
            return adapter._load_shd(stem)


def register():
    """Enable ``model='pelagic'`` without replacing arlpy's default model preference."""
    import arlpy.uwapm as pm
    if not hasattr(pm, "_models") or not hasattr(pm._Bellhop, "_create_env_file"):
        raise RuntimeError("unsupported arlpy model interface; expected arlpy 1.9")
    existing = [model for name, model in pm._models if name == "pelagic"]
    if existing and existing != [_Pelagic]:
        raise RuntimeError("another arlpy backend already uses the name pelagic")
    if not existing:
        pm._models.append(("pelagic", _Pelagic))
