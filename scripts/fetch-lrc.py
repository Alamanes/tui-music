#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""tui-music-fetch-lrc — 批量下载 LRC 同步歌词

用法：
  tui-music-fetch-lrc [路径...]        路径可以是音频文件或目录（可多个）；缺省扫描 ~/Music
  tui-music-fetch-lrc --dry-run       只报告哪些歌缺歌词，不下载
  tui-music-fetch-lrc --limit N       最多处理 N 首（调试用）
  tui-music-fetch-lrc --force         无视已存在的同名 .lrc 重新拉取

数据源（按优先级）：
  1. LRCLIB (https://lrclib.net) —— 免费公共歌词库，无鉴权
  2. 网易云音乐 API —— 中文歌兜底（未登录接口，偶发风控时自动重试）

歌词保存为与音频同目录的同名 .lrc，tui-music 播放时会自动读取。
仅下载带时间戳的同步歌词；纯文本歌词会跳过。
"""

import argparse
import difflib
import json
import os
import re
import sys
import time
import urllib.parse
import urllib.request

AUDIO_EXTS = {".mp3", ".flac", ".wav", ".ogg", ".m4a", ".aac", ".opus", ".aiff", ".aif", ".alac"}
UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36"
OFFSET_TAG = re.compile(r"\[\s*offset\s*:[^\]]*\]", re.I)
TIME_TAG = re.compile(r"\[\d{1,2}:\d{2}([.:]\d+)?\]")


def http_get(url: str, timeout: float = 8.0) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return resp.read()


def collect_audio(paths: list[str]) -> list[str]:
    out: list[str] = []
    for p in paths:
        if os.path.isdir(p):
            for root, _, files in os.walk(p):
                for f in sorted(files):
                    if os.path.splitext(f)[1].lower() in AUDIO_EXTS:
                        out.append(os.path.join(root, f))
        elif os.path.splitext(p)[1].lower() in AUDIO_EXTS:
            out.append(p)
    return out


def similarity(query: str, cand: str) -> float:
    return difflib.SequenceMatcher(None, query.lower(), cand.lower()).ratio()


def pick_best(query: str, items: list[dict], threshold: float = 0.35):
    """从候选里挑出相似度最高的一条；低于阈值视为不匹配。"""
    best, best_score = None, 0.0
    for it in items:
        name = it.get("name") or ""
        artist = it.get("artist") or ""
        if not isinstance(artist, str):
            artist = " ".join(artist)
        score = similarity(query, f"{name} {artist}".strip())
        if score > best_score:
            best_score, best = score, it
    if best is None or best_score < threshold:
        return None
    return best


def fetch_lrclib(query: str) -> str | None:
    """LRCLIB：/api/search?q=... 返回候选，取 syncedLyrics 非空且最相似者。"""
    url = "https://lrclib.net/api/search?" + urllib.parse.urlencode({"q": query})
    try:
        data = json.loads(http_get(url))
    except Exception:
        return None
    if not isinstance(data, list):
        return None
    items = [i for i in data if i.get("syncedLyrics")]
    if not items:
        return None
    hit = pick_best(query, items)
    if hit is None:
        return None
    lrc = (hit.get("syncedLyrics") or "").strip()
    return lrc or None


def fetch_netease(query: str) -> str | None:
    """网易云：搜索歌曲 → 歌词接口。返回 LRC 文本或 None。
    接口偶发风控（空结果），最多重试 3 次。"""
    search_url = "https://music.163.com/api/search/get/web?" + urllib.parse.urlencode(
        {"s": query, "type": 1, "limit": 5, "offset": 0}
    )
    songs = []
    for _ in range(3):
        try:
            data = json.loads(http_get(search_url))
            songs = ((data.get("result") or {}).get("songs")) or []
        except Exception:
            songs = []
        if songs:
            break
        time.sleep(1.5)
    if not songs:
        return None
    ranked = sorted(
        songs,
        key=lambda s: similarity(
            query,
            f"{s.get('name','')} {' '.join(a.get('name','') for a in s.get('artists') or [])}".strip(),
        ),
        reverse=True,
    )
    if not ranked:
        return None
    song_id = ranked[0]["id"]
    lyric_url = "https://music.163.com/api/song/lyric?" + urllib.parse.urlencode(
        {"id": song_id, "lv": 1, "kv": 1, "tv": -1}
    )
    lrc = ""
    for _ in range(3):
        try:
            ldata = json.loads(http_get(lyric_url))
            lrc = ((ldata.get("lrc") or {}).get("lyric")) or ""
        except Exception:
            lrc = ""
        if lrc:
            break
        time.sleep(1.5)
    if not lrc:
        return None
    lrc = OFFSET_TAG.sub("", lrc).strip()
    if not lrc or not TIME_TAG.search(lrc):
        return None
    return lrc


def candidate_queries(stem: str) -> list[str]:
    """从文件名生成候选搜索词：原始 → 去【翻唱标记】→ 「歌手 - 歌名」取歌名 →
    去末尾歌手段（「歌名 歌手」格式）兜底。"""
    qs = [stem]
    cleaned = re.sub(r"^【[^】]*】\s*", "", stem)
    if cleaned != stem:
        qs.append(cleaned)
    if " - " in cleaned:
        _, _, title = cleaned.partition(" - ")
        if title.strip():
            qs.append(title.strip())
    parts = cleaned.split()
    if len(parts) > 1:
        qs.append(" ".join(parts[:-1]))
    seen = set()
    return [q for q in qs if not (q in seen or seen.add(q))]


def fetch_lyrics(query: str) -> tuple[str | None, str]:
    for src, fn in (("LRCLIB", fetch_lrclib), ("网易云", fetch_netease)):
        try:
            lrc = fn(query)
        except Exception:
            lrc = None
        if lrc:
            return lrc, src
    return None, ""


def main() -> int:
    ap = argparse.ArgumentParser(description="批量下载 LRC 同步歌词（LRCLIB + 网易云兜底）")
    ap.add_argument("path", nargs="*", default=None, help="音频文件或目录（可多个）；缺省扫描 ~/Music")
    ap.add_argument("--dry-run", action="store_true", help="只报告缺歌词的歌，不下载")
    ap.add_argument("--limit", type=int, default=0, help="最多处理 N 首")
    ap.add_argument("--force", action="store_true", help="忽略已有 .lrc 重新拉取")
    args = ap.parse_args()

    paths = args.path or [os.path.expanduser("~/Music")]
    files = collect_audio(paths)
    if not files:
        print("未找到音频文件:", paths)
        return 1
    if args.limit:
        files = files[: args.limit]

    ok, skip, fail = 0, 0, 0
    base = paths[0] if len(paths) == 1 else os.getcwd()
    for i, audio in enumerate(files, 1):
        stem, _ = os.path.splitext(audio)
        lrc_path = stem + ".lrc"
        if os.path.exists(lrc_path) and not args.force:
            skip += 1
            print(f"[跳过] 已有歌词  {os.path.relpath(audio, base)}")
            continue
        query = os.path.basename(stem).strip()
        if not query:
            fail += 1
            print(f"[失败] 空文件名  {audio}")
            continue
        print(f"[{i}/{len(files)}] 查询: {query}")
        if args.dry_run:
            continue
        lrc, src = None, ""
        for cand in candidate_queries(query):
            lrc, src = fetch_lyrics(cand)
            if lrc:
                break
        if not lrc:
            fail += 1
            print("        → 未找到同步歌词")
            continue
        with open(lrc_path, "w", encoding="utf-8") as f:
            f.write(lrc.rstrip() + "\n")
        ok += 1
        print(f"        → 已保存 ({src}): {os.path.relpath(lrc_path, base)}")
        time.sleep(0.2)

    print(f"\n完成：成功 {ok}，跳过 {skip}，失败 {fail}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
