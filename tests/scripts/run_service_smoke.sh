#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
binary=${FOLIO_SERVICE_BIN:-"$repo/target/debug/folio-service"}
folio=${FOLIO_BIN:-"$repo/target/debug/folio"}
port=${FOLIOFORGE_SMOKE_PORT:-18080}
address="127.0.0.1:$port"
if [ ! -x "$binary" ] || [ ! -x "$folio" ]; then
    echo "build folio-cli and folio-service before running this smoke suite" >&2
    exit 2
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/folioforge-service.XXXXXX")
trap 'if [ -n "${server_pid:-}" ]; then kill "$server_pid" 2>/dev/null || true; fi; rm -rf "$work"' EXIT INT TERM
"$repo/tests/scripts/build_minimal_corpus.sh" "$work/corpus" >/dev/null
corpus="$work/corpus"
simple="$corpus/001-text.epub"
ruby="$corpus/014-ruby.epub"
"$folio" convert "$simple" --to kf7 --mode compatible --output "$work/seed.mobi" >/dev/null 2>"$work/seed-kf7.log"
"$folio" convert "$simple" --to kf8 --mode compatible --output "$work/seed.azw3" >/dev/null 2>"$work/seed-kf8.log"
"$folio" convert "$simple" --to kfx --mode compatible --output "$work/seed.kfx" >/dev/null 2>"$work/seed-kfx.log"

FOLIOFORGE_BIND="$address" FOLIOFORGE_WORK_DIR="$work/service-work" FOLIOFORGE_MAX_FILES=4 FOLIOFORGE_ONLINE_ENABLED=0 "$binary" >"$work/service.log" 2>&1 &
server_pid=$!
i=0
until curl -fsS "http://$address/health" >/dev/null 2>&1; do
    i=$((i + 1))
    [ "$i" -lt 40 ] || { sed -n '1,120p' "$work/service.log" >&2; exit 1; }
    sleep 0.25
done

submit_job() {
    input=$1
    filename=$2
    target=$3
    mode=$4
    expected_status=$5
    batch_mode=${6:-best_effort}
    response=$(curl -fsS -X POST "http://$address/convert" \
        -F "files[]=@$input;filename=$filename" \
        -F "target=$target" -F "mode=$mode" -F "batch_mode=$batch_mode")
    job_id=$(printf '%s' "$response" | python3 -c 'import json,sys; print(json.load(sys.stdin)["job_id"])')
    i=0
    status=queued
    while [ "$status" = queued ] || [ "$status" = running ]; do
        sleep 0.1
        state=$(curl -fsS "http://$address/jobs/$job_id")
        status=$(printf '%s' "$state" | python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])')
        i=$((i + 1))
        [ "$i" -lt 300 ] || { echo "job timeout: $job_id" >&2; exit 1; }
    done
    [ "$status" = "$expected_status" ] || {
        echo "unexpected job status $status for $input -> $target ($mode), expected $expected_status" >&2
        printf '%s\n' "$state" >&2
        exit 1
    }
    if [ "$status" = completed ]; then
        download="$work/download-$job_id"
        curl -fsS "http://$address/jobs/$job_id/events" | grep -q 'event: progress'
        curl -fsS "http://$address/jobs/$job_id/download" -o "$download"
        [ -s "$download" ] || { echo "empty download for $job_id" >&2; exit 1; }
        printf '%s\n' "$state" | python3 -c '
import json,sys
state=json.load(sys.stdin)
report=state["report"]["items"][0]["report"]
assert report["round_trip"]["checked"] and report["round_trip"]["passed"]
'
    fi
    printf 'ok service %s -> %s (%s): %s\n' "$filename" "$target" "$mode" "$status"
}

curl -fsS "http://$address/" | grep -q 'Book Edit Plan'
capabilities=$(curl -fsS "http://$address/capabilities")
printf '%s\n' "$capabilities" | python3 -c '
import json,sys
value=json.load(sys.stdin)
formats={item["format"] for item in value["input_formats"]}
assert {"Markdown", "HTML", "HTMLZ", "FB2", "DOCX"}.issubset(formats)
assert "EPUB" in formats and "TXT" in formats
'
printf 'ok service dynamic format capabilities\n'
online_status=$(curl -fsS "http://$address/online/status")
printf '%s\n' "$online_status" | python3 -c 'import json,sys; value=json.load(sys.stdin); assert value["enabled"] is False and value["core_offline"] is True'
online_code=$(curl -sS -o "$work/online-disabled.json" -w '%{http_code}' -X POST "http://$address/online/metadata/search" -H 'content-type: application/json' --data '{"query":"Pride and Prejudice"}')
[ "$online_code" = 503 ] || { echo "online provider returned $online_code while disabled, expected 503" >&2; exit 1; }
curl -fsS "http://$address/logo.png" -o "$work/logo.png"
cmp "$repo/assets/folioforge-logo.png" "$work/logo.png"
printf 'ok service home and exact logo asset\n'
preflight=$(curl -fsS -X POST "http://$address/preflight" \
    -F "files[]=@$ruby;filename=Ruby.epub" -F "target=kf7" -F "mode=compatible")
printf '%s\n' "$preflight" | python3 -c '
import json,sys
value=json.load(sys.stdin)
assert value["summary"]["total"] == 1
assert value["summary"]["failed"] == 0
assert value["files"][0]["input_report"]["detected_format"] == "EPUB"
assert value["files"][0]["semantic_report"]["valid"] is True
'
printf 'ok service preflight\n'

edit_json='{"metadata":{"title":"Service Smoke","clear_fields":[]},"cover":"Keep","typography":{"line_height":"1.7"},"fonts":{"strip_embedded_fonts":false},"styles":[{"css":"color: #333"}],"structure":{"remove_navigation":false}}'
preview=$(curl -fsS -X POST "http://$address/preview" \
    -F "files[]=@$simple;filename=Preview.epub" \
    -F "cover=@$repo/assets/folioforge-logo.png;filename=cover.png;type=image/png" \
    -F "cover_fit=Fit" -F "edit=$edit_json" -F "target=epub" -F "mode=compatible" \
    -F "preview_device=phone" -F "preview_orientation=landscape" -F "preview_font_size_percent=150")
printf '%s\n' "$preview" | python3 -c '
import json,sys
value=json.load(sys.stdin)
assert value["source_title"] == "Service Smoke"
assert "color: #333" in value["html"]
assert "not a Kindle Exact Preview" in value["html"]
assert value["blocked"] is False
assert value["target"]["device"] == "phone"
assert value["target"]["orientation"] == "landscape"
assert value["target"]["viewport_width"] == 844
assert value["target"]["font_size_percent"] == 150
'
printf 'ok service BookEditPlan preview and cover upload\n'

limit_status=$(curl -sS -o "$work/file-limit.json" -w '%{http_code}' -X POST "http://$address/preflight" \
    -F "files[]=@$simple;filename=1.epub" \
    -F "files[]=@$simple;filename=2.epub" \
    -F "files[]=@$simple;filename=3.epub" \
    -F "files[]=@$simple;filename=4.epub" \
    -F "files[]=@$simple;filename=5.epub")
[ "$limit_status" = 400 ] || { echo "upload count limit returned $limit_status, expected 400" >&2; exit 1; }
printf 'ok service upload file-count limit\n'

submit_job "$simple" "Books/Simple.epub" kf7 strict completed
submit_job "$ruby" "Books/Ruby.epub" kf7 compatible completed
submit_job "$ruby" "Books/Ruby-readable.epub" kf7 readable completed
submit_job "$ruby" "Books/Ruby-strict.epub" kf7 strict completed_with_errors
submit_job "$work/seed.mobi" "Books/Seed.mobi" epub compatible completed
submit_job "$work/seed.azw3" "Books/Seed.azw3" kf7 compatible completed
submit_job "$work/seed.kfx" "Books/Seed.kfx" kf7 compatible completed
submit_job "$work/seed.kfx" "Books/Seed.kfx" epub compatible completed

strict_batch=$(curl -fsS -X POST "http://$address/convert" \
    -F "files[]=@$simple;filename=Books/A.epub" \
    -F "files[]=@$ruby;filename=Books/B.epub" \
    -F "target=kf7" -F "mode=strict" -F "batch_mode=strict")
strict_batch_id=$(printf '%s' "$strict_batch" | python3 -c 'import json,sys; print(json.load(sys.stdin)["job_id"])')
i=0
status=queued
while [ "$status" = queued ] || [ "$status" = running ]; do
    sleep 0.1
    state=$(curl -fsS "http://$address/jobs/$strict_batch_id")
    status=$(printf '%s' "$state" | python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])')
    i=$((i + 1))
    [ "$i" -lt 300 ] || exit 1
done
[ "$status" = failed ] || { printf '%s\n' "$state" >&2; exit 1; }
printf '%s\n' "$state" | python3 -c '
import json,sys
report=json.load(sys.stdin)["report"]
assert report["aborted"] is True
assert report["succeeded"] == 0
'
printf 'ok service strict batch rollback\n'

multi=$(curl -fsS -X POST "http://$address/convert" \
    -F "files[]=@$simple;filename=Books/A.epub" \
    -F "files[]=@$ruby;filename=Books/Series/B.epub" \
    -F "target=kfx" -F "mode=compatible" -F "batch_mode=best_effort")
multi_id=$(printf '%s' "$multi" | python3 -c 'import json,sys; print(json.load(sys.stdin)["job_id"])')
i=0
status=queued
while [ "$status" = queued ] || [ "$status" = running ]; do
    sleep 0.1
    state=$(curl -fsS "http://$address/jobs/$multi_id")
    status=$(printf '%s' "$state" | python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])')
    i=$((i + 1))
    [ "$i" -lt 300 ] || exit 1
done
[ "$status" = completed ] || { printf '%s\n' "$state" >&2; exit 1; }
curl -fsS "http://$address/jobs/$multi_id/download" -o "$work/multiple.zip"
unzip -Z1 "$work/multiple.zip" | grep -qx 'FolioForge-report.json'
unzip -Z1 "$work/multiple.zip" | grep -q 'A.kfx'
unzip -Z1 "$work/multiple.zip" | grep -q 'Series/B.kfx'
printf 'ok service folder upload and ZIP report\n'

printf 'service smoke passed: single, preflight, all modes, four input formats, folder upload, SSE, download, ZIP\n'
