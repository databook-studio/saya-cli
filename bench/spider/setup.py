#!/usr/bin/env python3
"""Prepares a Spider benchmark corpus under `.bench/spider`.

What this fetches for you, from the benchmarks' own repositories:

  * Spider 2.0-lite questions and evaluation metadata
  * the multi-variant gold result tables, on demand during scoring
  * the reference documents Spider 2.0 supplies with 55 of its questions
  * the map from a Spider 2.0 database name to the real BigQuery
    `project.dataset` that holds it — read out of the repository tree, since
    that mapping exists only as directory names

What it cannot fetch, and why: Spider 1.0's dev set and Spider 2.0's local
SQLite databases are distributed as large archives behind interstitials rather
than as plain URLs. Point `--import-from` at a directory that already holds
them, or drop them in by hand; this script tells you exactly what is missing
and where it expects to find it.

Nothing here writes into the user's own saya configuration: profiles are
generated into the corpus directory and selected with `SAYA_CONFIG_HOME`.
"""
import argparse, json, os, shutil, sys, urllib.parse, urllib.request

ROOT = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(ROOT, "..", ".."))
CORPUS = os.path.join(REPO, ".bench", "spider")

RAW = "https://raw.githubusercontent.com/xlang-ai/Spider2/main"
TREE = "https://api.github.com/repos/xlang-ai/Spider2/git/trees/main?recursive=1"
LITE = f"{RAW}/spider2-lite/spider2-lite.jsonl"
EVAL = f"{RAW}/spider2-lite/evaluation_suite/gold/spider2lite_eval.jsonl"
UTILS = f"{RAW}/spider2-lite/evaluation_suite/evaluate_utils.py"
DOCS = f"{RAW}/spider2-lite/resource/documents"

# Where the two hand-supplied corpora are expected to land.
NEEDED = {
    "spider1": ("spider_data/dev.json",
                "Spider 1.0 dev split — the `spider_data` directory from the "
                "Spider 1.0 release (dev.json plus database/<db>/<db>.sqlite)"),
    "spider2_local": ("s2local",
                      "Spider 2.0-lite local SQLite databases — the directory "
                      "of *.sqlite files from the Spider 2.0-lite release"),
}


def get(url, dest, quiet=False):
    if os.path.exists(dest) and os.path.getsize(dest) > 0:
        return True
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    try:
        urllib.request.urlretrieve(url, dest)
        return True
    except Exception as exc:
        if not quiet:
            print(f"  ! could not fetch {url}: {exc}")
        if os.path.exists(dest):
            os.remove(dest)
        return False


def fetch_questions():
    print("questions and evaluation metadata")
    ok = get(LITE, f"{CORPUS}/spider2-lite.jsonl")
    get(EVAL, f"{CORPUS}/spider2lite_eval.jsonl")
    # The comparator is the benchmark's own; using ours would not be its metric.
    get(UTILS, f"{CORPUS}/evaluate_utils.py")
    if ok:
        n = sum(1 for _ in open(f"{CORPUS}/spider2-lite.jsonl"))
        print(f"  spider2-lite: {n} questions")


def fetch_documents():
    """The reference document Spider 2.0 ships with some questions. Its
    protocol gives these to the model, so withholding them would measure a
    harder benchmark than the published one."""
    path = f"{CORPUS}/spider2-lite.jsonl"
    if not os.path.exists(path):
        return
    names = sorted({json.loads(l)["external_knowledge"]
                    for l in open(path) if l.strip()
                    and json.loads(l).get("external_knowledge")})
    got = sum(get(f"{DOCS}/{urllib.parse.quote(n)}", f"{CORPUS}/documents/{n}",
                  quiet=True) for n in names)
    print(f"reference documents: {got}/{len(names)}")


def fetch_bigquery_map():
    """Spider 2.0 records which BigQuery project owns each database only as a
    directory name (`.../bigquery/<db>/<project.dataset>/`), so the mapping is
    read out of the repository tree rather than a manifest."""
    dest = f"{CORPUS}/bigquery_datasets.json"
    if os.path.exists(dest):
        print("bigquery dataset map: cached")
        return
    try:
        with urllib.request.urlopen(TREE, timeout=120) as r:
            tree = json.load(r)
    except Exception as exc:
        print(f"  ! could not read the Spider2 tree: {exc}")
        return
    prefix = "spider2-lite/resource/databases/bigquery/"
    mapping = {}
    for entry in tree.get("tree", []):
        p = entry["path"]
        if not p.startswith(prefix):
            continue
        parts = p[len(prefix):].split("/")
        if len(parts) >= 2 and "." in parts[1]:
            mapping.setdefault(parts[0], set()).add(parts[1])
    out = {k: sorted(v) for k, v in sorted(mapping.items())}
    json.dump(out, open(dest, "w"), indent=1)
    spanning = sum(1 for v in out.values() if len(v) > 1)
    print(f"bigquery dataset map: {len(out)} databases "
          f"({spanning} span more than one dataset)")


def import_from(source):
    """Copies an already-prepared corpus in, so a machine that has the large
    archives can seed one that does not."""
    for key, (rel, _) in NEEDED.items():
        src = os.path.join(source, rel.split("/")[0])
        dst = os.path.join(CORPUS, rel.split("/")[0])
        if os.path.exists(src) and not os.path.exists(dst):
            print(f"importing {rel.split('/')[0]} …")
            shutil.copytree(src, dst)


def report_missing():
    missing = []
    for key, (rel, why) in NEEDED.items():
        if not os.path.exists(os.path.join(CORPUS, rel)):
            missing.append((rel, why))
    if not missing:
        print("\nall corpora present")
        return 0
    print("\nstill missing — supply these by hand:")
    for rel, why in missing:
        print(f"  {os.path.join(CORPUS, rel)}\n      {why}")
    return 1


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--import-from", metavar="DIR",
                    help="a directory already holding spider_data/ and s2local/")
    args = ap.parse_args()
    os.makedirs(CORPUS, exist_ok=True)
    print(f"corpus: {CORPUS}\n")
    fetch_questions()
    fetch_documents()
    fetch_bigquery_map()
    if args.import_from:
        import_from(args.import_from)
    code = report_missing()
    print("\nnext: python bench/spider/profiles.py   (writes saya connection profiles)")
    return code


if __name__ == "__main__":
    sys.exit(main())
