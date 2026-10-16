#!/usr/bin/env bash

# Reset one Rust Lankide workspace to a clean local conversation state.
#
# This is a destructive test-workspace tool. It preserves Workspace and Agent
# configuration, Provider credentials/state, and configured avatar resources;
# it moves the selected workspace's recoverable history and Harness state into
# a private backup before clearing the live state.

set -euo pipefail

program_name="$(basename "$0")"
home_arg=""
workspace_arg=""
backup_arg=""
dry_run=0
restart=1
confirmed=0

user_home=""
default_lankide_home=""
lankide_home=""
workspaces_root=""
workspace_root=""
workspace_id=""
database=""
machine_root=""
machine_state=""
state_existed=0
backup_root=""
service_domain=""

agent_records_json="[]"
agent_ids=""
target_execution_ids_json="[]"
keep_resource_ids_json="[]"
keep_resource_ids=""
keep_resource_names=""
config_before_hash=""
resource_config_before_hash=""
temporary_state=""

usage() {
  cat <<'EOF'
Usage:
  ./scripts/reset-agent-context.sh [options]

Reset one Rust Lankide workspace so the next Agent turn starts without local
conversation, unread, execution, tool, attachment, or Harness session history.

Options:
  -H, --home PATH       Lankide home to reset. If omitted, use LANKIDE_HOME or
                        ~/.lankide.
  -w, --workspace ID|PATH
                        Workspace ID or workspace directory. If omitted, use
                        LANKIDE_WORKSPACE_ID, the current workspace directory,
                        or the only workspace in the local database.
      --backup-dir PATH Store the recoverable backup under this directory.
      --restart          Start Runtime and Machine again and verify that old
                        local history is not reintroduced (default).
      --no-restart       Leave Runtime and Machine stopped after the reset.
      --dry-run          Resolve and report the target without changing files.
  -y, --yes             Skip the destructive-operation confirmation prompt.
  -h, --help            Show this help.

Environment:
  LANKIDE_HOME            Override the local Rust Lankide home.
  LANKIDE_WORKSPACE_ID    Override the default workspace selection.

The default mode restarts the services after the reset. Provider configuration,
credentials, identity state, Provider cursors, Workspace/Agent configuration,
and configured avatar resources are preserved. Normal Pi, Codex, and Claude
user homes are not touched. Use --no-restart for an intentionally stopped
test or when using a custom --home.
EOF
}

die() {
  if [ -n "$backup_root" ]; then
    printf 'ERROR: %s\nRecoverable backup: %s\n' "$*" "$backup_root" >&2
  else
    printf 'ERROR: %s\n' "$*" >&2
  fi
  exit 1
}

cleanup() {
  if [ -n "$temporary_state" ]; then
    rm -f "$temporary_state"
  fi
}

trap cleanup EXIT

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    die "required command not found: $1"
  fi
}

canonical_directory() {
  if [ ! -d "$1" ]; then
    die "directory does not exist: $1"
  fi
  (cd "$1" && pwd -P)
}

reject_symlink() {
  if [ -L "$1" ]; then
    die "refusing to touch a symbolic link: $1"
  fi
}

valid_identifier() {
  [[ "$1" =~ ^[a-z][a-z0-9_]*$ ]]
}

resolve_user_home() {
  local candidate user_name
  user_name="$(id -un)"
  candidate=""
  if [ -n "${HOME:-}" ] && [ -d "${HOME}" ]; then
    candidate="${HOME}"
  fi
  if [ -z "$candidate" ] && command -v dscl >/dev/null 2>&1; then
    candidate="$(dscl . -read "/Users/$user_name" NFSHomeDirectory 2>/dev/null | awk '{print $2}')"
  fi
  if [ -z "$candidate" ] && command -v getent >/dev/null 2>&1; then
    candidate="$(getent passwd "$(id -u)" 2>/dev/null | awk -F: '{print $6}')"
  fi
  if [ -z "$candidate" ] && command -v python3 >/dev/null 2>&1; then
    candidate="$(python3 -c 'import os; print(os.path.expanduser("~"))')"
  fi
  [ -n "$candidate" ] || die "could not resolve the current user's home directory"
  user_home="$(canonical_directory "$candidate")"
}

parse_arguments() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -H|--home)
        [ "$#" -ge 2 ] || die "$1 requires a path"
        home_arg="$2"
        shift 2
        ;;
      --home=*)
        home_arg="${1#*=}"
        shift
        ;;
      -w|--workspace)
        [ "$#" -ge 2 ] || die "$1 requires an ID or path"
        workspace_arg="$2"
        shift 2
        ;;
      --workspace=*)
        workspace_arg="${1#*=}"
        shift
        ;;
      --backup-dir)
        [ "$#" -ge 2 ] || die "$1 requires a path"
        backup_arg="$2"
        shift 2
        ;;
      --backup-dir=*)
        backup_arg="${1#*=}"
        shift
        ;;
      --restart)
        restart=1
        shift
        ;;
      --no-restart)
        restart=0
        shift
        ;;
      --dry-run)
        dry_run=1
        shift
        ;;
      -y|--yes)
        confirmed=1
        shift
        ;;
      -h|--help|help)
        usage
        exit 0
        ;;
      --)
        shift
        [ "$#" -eq 0 ] || die "unexpected positional arguments: $*"
        ;;
      *)
        die "unknown argument: $1 (run $program_name --help)"
        ;;
    esac
  done
}

resolve_lankide_home() {
  local candidate
  if [ -n "$home_arg" ]; then
    candidate="$home_arg"
  elif [ -n "${LANKIDE_HOME:-}" ]; then
    candidate="${LANKIDE_HOME}"
  else
    candidate="$user_home/.lankide"
  fi
  reject_symlink "$candidate"
  lankide_home="$(canonical_directory "$candidate")"
  default_lankide_home="$user_home/.lankide"
  if [ -d "$default_lankide_home" ]; then
    reject_symlink "$default_lankide_home"
    default_lankide_home="$(canonical_directory "$default_lankide_home")"
  fi
}

resolve_workspace() {
  local selected current candidate ids count
  workspaces_root="$lankide_home/workspaces"
  reject_symlink "$workspaces_root"
  workspaces_root="$(canonical_directory "$workspaces_root")"

  selected="$workspace_arg"
  if [ -z "$selected" ] && [ -n "${LANKIDE_WORKSPACE_ID:-}" ]; then
    selected="${LANKIDE_WORKSPACE_ID}"
  fi

  if [ -n "$selected" ]; then
    if valid_identifier "$selected"; then
      workspace_id="$selected"
      workspace_root="$workspaces_root/$workspace_id"
    else
      reject_symlink "$selected"
      workspace_root="$(canonical_directory "$selected")"
      workspace_id="$(basename "$workspace_root")"
    fi
  else
    current="$(pwd -P)"
    if [ "$(dirname "$current")" = "$workspaces_root" ] && valid_identifier "$(basename "$current")"; then
      workspace_root="$current"
      workspace_id="$(basename "$current")"
    else
      ids="$(sqlite3 -readonly "$database" "SELECT workspace_id FROM workspaces ORDER BY workspace_id;" 2>/dev/null || true)"
      count="$(printf '%s\n' "$ids" | awk 'NF { count++ } END { print count + 0 }')"
      [ "$count" -eq 1 ] || die "multiple or no workspaces are configured; pass --workspace ID"
      workspace_id="$(printf '%s\n' "$ids" | awk 'NF { print; exit }')"
      workspace_root="$workspaces_root/$workspace_id"
    fi
  fi

  valid_identifier "$workspace_id" || die "invalid workspace ID: $workspace_id"
  reject_symlink "$workspace_root"
  workspace_root="$(canonical_directory "$workspace_root")"
  [ "$(dirname "$workspace_root")" = "$workspaces_root" ] || die "workspace must be a direct child of $workspaces_root"
  [ "$(basename "$workspace_root")" = "$workspace_id" ] || die "workspace directory name does not match its ID"
}

resolve_database() {
  database="$lankide_home/runtime.db"
  [ -f "$database" ] || die "Runtime database not found: $database"
  reject_symlink "$database"
}

resolve_machine_state() {
  machine_root="$workspaces_root/.machine"
  if [ -e "$machine_root" ] || [ -L "$machine_root" ]; then
    reject_symlink "$machine_root"
    [ -d "$machine_root" ] || die "Machine state directory is not a directory: $machine_root"
  fi
  machine_state="$machine_root/state.json"
  if [ -e "$machine_state" ] || [ -L "$machine_state" ]; then
    reject_symlink "$machine_state"
    [ -f "$machine_state" ] || die "Machine state is not a regular file: $machine_state"
    state_existed=1
  fi
}

validate_database() {
  local schema_version integrity table exists workspace_count
  if ! schema_version="$(sqlite3 -readonly "$database" "PRAGMA user_version;" 2>/dev/null)"; then
    die "could not read Runtime database: $database"
  fi
  [ "$schema_version" = "1" ] || die "unsupported Runtime database schema $schema_version; this reset contract requires schema 1"

  if ! integrity="$(sqlite3 -readonly "$database" "PRAGMA integrity_check;" 2>/dev/null)"; then
    die "could not check Runtime database integrity: $database"
  fi
  [ "$integrity" = "ok" ] || die "Runtime database failed PRAGMA integrity_check: $integrity"

  for table in \
    workspaces workspace_named_secrets agent_named_secrets provider_kv \
    resource_metadata workspace_resource_counters agents agent_file_deletions people channels chats messages reactions \
    agent_inbox_items agent_chat_cursors agent_chat_attention unread_snapshots \
    unread_snapshot_chats unread_snapshot_messages unread_snapshot_triggers executions \
    execution_attempts execution_events steer_deliveries execution_attempt_authorizations \
    execution_attempt_capabilities execution_deliveries execution_delivery_items \
    execution_delivery_chats message_send_attempts tool_invocations
  do
    exists="$(sqlite3 -readonly "$database" "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='$table');")"
    [ "$exists" = "1" ] || die "Runtime database is missing required table: $table"
  done

  workspace_count="$(sqlite3 -readonly "$database" "SELECT count(*) FROM workspaces WHERE workspace_id='$workspace_id';")"
  [ "$workspace_count" = "1" ] || die "workspace $workspace_id is not present in $database"

  if [ "$state_existed" -eq 1 ]; then
    jq -e 'type == "object"' "$machine_state" >/dev/null || die "Machine state is not valid JSON: $machine_state"
  fi
}

load_workspace_records() {
  local avatar_values_json avatar resource_name
  agent_records_json="$(sqlite3 -readonly -json "$database" \
    "SELECT agent_id,harness_id,env_json FROM agents WHERE workspace_id='$workspace_id' ORDER BY agent_id;")"
  printf '%s' "$agent_records_json" | jq -e 'type == "array" and all(.[]; (.agent_id | type) == "string" and (.harness_id | type) == "string" and (.env_json | type) == "string")' >/dev/null \
    || die "Agent configuration is not readable for workspace $workspace_id"
  agent_ids="$(printf '%s' "$agent_records_json" | jq -r '.[].agent_id')"

  target_execution_ids_json="$(sqlite3 -readonly -json "$database" \
    "SELECT execution_id FROM executions WHERE workspace_id='$workspace_id' ORDER BY execution_id;" | jq -c '[.[].execution_id]')"

  keep_resource_ids_json="$(sqlite3 -readonly -json "$database" \
    "SELECT DISTINCT resource_id FROM resource_metadata WHERE workspace_id='$workspace_id' AND (resource_id=(SELECT cover_resource_id FROM workspaces WHERE workspace_id='$workspace_id') OR resource_id IN (SELECT CAST(avatar AS INTEGER) FROM agents WHERE workspace_id='$workspace_id' AND avatar IS NOT NULL AND avatar <> '' AND avatar NOT GLOB '*[^0-9]*')) ORDER BY resource_id;" | jq -c '[.[].resource_id]')"
  keep_resource_ids="$(printf '%s' "$keep_resource_ids_json" | jq -r '.[]')"
  keep_resource_names="$keep_resource_ids"
  avatar_values_json="$(sqlite3 -readonly -json "$database" \
    "SELECT DISTINCT avatar FROM agents WHERE workspace_id='$workspace_id' AND avatar IS NOT NULL AND avatar <> '' ORDER BY avatar;")"
  while IFS= read -r avatar; do
    [ -n "$avatar" ] || continue
    case "$avatar" in
      [0-9]*)
        resource_name="$avatar"
        ;;
      "resources/$workspace_id/"*)
        resource_name="${avatar#resources/$workspace_id/}"
        case "$resource_name" in
          ""|*/*|.|..)
            die "invalid Agent avatar resource path: $avatar"
            ;;
        esac
        ;;
      *)
        die "Agent avatar resource is outside the selected workspace: $avatar"
        ;;
    esac
    keep_resource_names="${keep_resource_names}${keep_resource_names:+$'\n'}$resource_name"
  done < <(printf '%s' "$avatar_values_json" | jq -r '.[].avatar')
}

validate_agent_paths() {
  local record agent_id env_json key value agent_home
  while IFS= read -r record; do
    agent_id="$(printf '%s' "$record" | jq -r '.agent_id')"
    valid_identifier "$agent_id" || die "invalid Agent ID in database: $agent_id"
    agent_home="$workspace_root/agents/$agent_id"
    if [ -e "$agent_home" ] || [ -L "$agent_home" ]; then
      reject_symlink "$agent_home"
      [ -d "$agent_home" ] || die "Agent home is not a directory: $agent_home"
    fi
    env_json="$(printf '%s' "$record" | jq -r '.env_json')"
    printf '%s' "$env_json" | jq -e 'type == "object"' >/dev/null || die "Agent $agent_id has invalid env_json"

    for key in HOME CODEX_HOME PI_CODING_AGENT_DIR PI_CODING_AGENT_SESSION_DIR \
      LANKIDE_PI_ARTIFACT_DIR LANKIDE_PI_PID_JOURNAL LANKIDE_CLAUDE_CONFIG_DIR \
      LANKIDE_CLAUDE_ARTIFACT_DIR
    do
      value="$(printf '%s' "$env_json" | jq -r --arg key "$key" 'if .[$key] == null then "" elif (.[$key] | type) == "string" then .[$key] else error("not a string") end')" \
        || die "Agent $agent_id has invalid path override $key"
      [ -n "$value" ] || continue
      [ -n "$agent_home" ] || die "Agent $agent_id has a path override but no Agent home"
      case "$value" in
        /*) ;;
        *) die "Agent $agent_id uses a relative path override $key; refusing to reset an ambiguous session path" ;;
      esac
      case "$value" in
        "$agent_home/.lankide"|"$agent_home/.lankide"/*) ;;
        *) die "Agent $agent_id uses $key outside its isolated Harness directory; refusing to reset" ;;
      esac
    done
  done < <(printf '%s' "$agent_records_json" | jq -c '.[]')
}

validate_filesystem_scope() {
  local resources_root target_resources agents_root agent_id agent_home path
  resources_root="$lankide_home/resources"
  if [ -e "$resources_root" ] || [ -L "$resources_root" ]; then
    reject_symlink "$resources_root"
    [ -d "$resources_root" ] || die "resource root is not a directory: $resources_root"
  fi
  target_resources="$resources_root/$workspace_id"
  if [ -e "$target_resources" ] || [ -L "$target_resources" ]; then
    reject_symlink "$target_resources"
    [ -d "$target_resources" ] || die "workspace resource root is not a directory: $target_resources"
  fi

  agents_root="$workspace_root/agents"
  if [ -e "$agents_root" ] || [ -L "$agents_root" ]; then
    reject_symlink "$agents_root"
    [ -d "$agents_root" ] || die "Agent root is not a directory: $agents_root"
  fi

  for agent_id in $agent_ids; do
    agent_home="$workspace_root/agents/$agent_id"
    for path in "$agent_home/.lankide" "$agent_home/resources" "$agent_home/tool-results"; do
      if [ -e "$path" ] || [ -L "$path" ]; then
        reject_symlink "$path"
      fi
    done
  done
}

workspace_sql() {
  printf "'%s'" "$workspace_id"
}

configuration_hash() {
  sqlite3 -readonly "$database" <<'SQL' | shasum -a 256 | awk '{print $1}'
.dump workspaces
.dump workspace_named_secrets
.dump agent_named_secrets
.dump provider_kv
.dump workspace_resource_counters
.dump agents
.dump people
SQL
}

resource_configuration_hash() {
  local resource_ids_sql
  resource_ids_sql="$(resource_ids_sql_value)"
  sqlite3 -readonly -separator $'\t' "$database" \
    "SELECT resource_id,file_name,media_type,size_bytes,sha256,storage_ref,created_at FROM resource_metadata WHERE workspace_id=$(workspace_sql) AND resource_id IN ($resource_ids_sql) ORDER BY resource_id;" \
    | shasum -a 256 | awk '{print $1}'
}

resource_ids_sql_value() {
  local ids_sql="" id
  while IFS= read -r id; do
    [ -n "$id" ] || continue
    [[ "$id" =~ ^[0-9]+$ ]] || die "invalid configured resource ID: $id"
    if [ -n "$ids_sql" ]; then
      ids_sql="$ids_sql,$id"
    else
      ids_sql="$id"
    fi
  done <<< "$keep_resource_ids"
  printf '%s' "${ids_sql:--1}"
}

db_count() {
  sqlite3 -readonly "$database" "$1"
}

print_counts() {
  local w resource_ids_sql
  w="$(workspace_sql)"
  resource_ids_sql="$(resource_ids_sql_value)"
  printf '  %-28s %s\n' 'channels' "$(db_count "SELECT count(*) FROM channels WHERE workspace_id=$w;")"
  printf '  %-28s %s\n' 'chats' "$(db_count "SELECT count(*) FROM chats WHERE workspace_id=$w;")"
  printf '  %-28s %s\n' 'messages' "$(db_count "SELECT count(*) FROM messages m JOIN chats c USING(chat_id) WHERE c.workspace_id=$w;")"
  printf '  %-28s %s\n' 'reactions' "$(db_count "SELECT count(*) FROM reactions WHERE workspace_id=$w;")"
  printf '  %-28s %s\n' 'inbox items' "$(db_count "SELECT count(*) FROM agent_inbox_items WHERE workspace_id=$w;")"
  printf '  %-28s %s\n' 'chat cursors' "$(db_count "SELECT count(*) FROM agent_chat_cursors WHERE workspace_id=$w;")"
  printf '  %-28s %s\n' 'unread snapshots' "$(db_count "SELECT count(*) FROM unread_snapshots WHERE workspace_id=$w;")"
  printf '  %-28s %s\n' 'executions' "$(db_count "SELECT count(*) FROM executions WHERE workspace_id=$w;")"
  printf '  %-28s %s\n' 'execution events' "$(db_count "SELECT count(*) FROM execution_events e JOIN executions x USING(execution_id) WHERE x.workspace_id=$w;")"
  printf '  %-28s %s\n' 'message send attempts' "$(db_count "SELECT count(*) FROM message_send_attempts a JOIN messages m USING(message_id) JOIN chats c USING(chat_id) WHERE c.workspace_id=$w;")"
  printf '  %-28s %s\n' 'tool invocations' "$(db_count "SELECT count(*) FROM tool_invocations t JOIN executions e USING(execution_id) WHERE e.workspace_id=$w;")"
  printf '  %-28s %s\n' 'removable resource records' "$(db_count "SELECT count(*) FROM resource_metadata WHERE workspace_id=$w AND resource_id NOT IN ($resource_ids_sql);")"
}

print_target() {
  printf 'Target workspace:          %s (%s)\n' "$workspace_id" "$(db_count "SELECT name FROM workspaces WHERE workspace_id=$(workspace_sql);")"
  printf 'Lankide home:              %s\n' "$lankide_home"
  printf 'Runtime database:          %s\n' "$database"
  printf 'Workspace directory:       %s\n' "$workspace_root"
  printf 'Machine state:            %s\n' "$machine_state"
  printf 'Harness user homes:        untouched\n'
  printf 'Preserved resource files:  %s\n' "${keep_resource_names:-none}"
  printf 'Runtime/Machine restart:  %s\n' "$([ "$restart" -eq 1 ] && printf 'yes' || printf 'no')"
  printf 'Current local state:\n'
  print_counts
}

service_home() {
  local service="$1"
  launchctl print "$service" 2>/dev/null | awk '
    /^[[:space:]]+--home[[:space:]]*$/ {
      if (getline > 0) {
        gsub(/^[[:space:]]+|[[:space:]]+$/, "")
        print
        exit
      }
    }
  '
}

validate_loaded_service_scope() {
  local service="$1" actual
  actual="$(service_home "$service")"
  if [ -z "$actual" ]; then
    actual="$default_lankide_home"
  else
    reject_symlink "$actual"
    actual="$(canonical_directory "$actual")"
  fi
  if [ "$actual" != "$lankide_home" ]; then
    [ "$lankide_home" != "$default_lankide_home" ] || die "loaded service $service points to a different Lankide home: $actual"
    return 1
  fi
}

lock_pid() {
  local lock="$1" contents pid run_id mode owner directory_owner
  [ -e "$lock" ] || return 1
  reject_symlink "$lock"
  [ -f "$lock" ] || die "daemon lock is not a regular file: $lock"
  mode="$(stat -f '%Lp' "$lock" 2>/dev/null || true)"
  owner="$(stat -f '%u' "$lock" 2>/dev/null || true)"
  mode="${mode#0}"
  directory_owner="$(stat -f '%u' "$(dirname "$lock")" 2>/dev/null || true)"
  [ "$mode" = "600" ] || die "daemon lock has unsafe permissions: $lock"
  [ "$owner" = "$(id -u)" ] && [ "$directory_owner" = "$(id -u)" ] || die "daemon lock is not owned by the current user: $lock"
  contents="$(<"$lock")"
  [[ "$contents" =~ ^pid=([0-9]+)[[:space:]]run_id=([^[:space:]]+)$ ]] || die "daemon lock has invalid contents: $lock"
  pid="${BASH_REMATCH[1]}"
  run_id="${BASH_REMATCH[2]}"
  [ -n "$run_id" ] || die "daemon lock has no run ID: $lock"
  printf '%s\n' "$pid"
}

stop_lock_process() {
  local lock="$1" binary="$2" pid command_line attempt
  [ -e "$lock" ] || return 0
  pid="$(lock_pid "$lock")"
  if ! kill -0 "$pid" 2>/dev/null; then
    return 0
  fi
  command_line="$(ps -p "$pid" -o command= 2>/dev/null || true)"
  case "$command_line" in
    *"$binary"*) ;;
    *) die "daemon lock $lock points to an unexpected process: $command_line" ;;
  esac
  kill -TERM "$pid" 2>/dev/null || die "could not stop process $pid for $lock"
  for attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50
  do
    if ! kill -0 "$pid" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done
  die "daemon process $pid did not stop after SIGTERM"
}

wait_for_lock_release() {
  local lock="$1" attempt
  for attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50
  do
    [ ! -e "$lock" ] && return 0
    sleep 0.1
  done
  return 0
}

stop_loaded_service() {
  local label="$1" service output
  service="$service_domain/$label"
  if ! launchctl print "$service" >/dev/null 2>&1; then
    return 0
  fi
  if ! validate_loaded_service_scope "$service"; then
    return 0
  fi
  output="$(launchctl bootout "$service" 2>&1)" || die "could not stop service $service: $output"
  case "$label" in
    com.lankide.daemon) wait_for_lock_release "$lankide_home/run/runtime.lock" ;;
    com.lankide.machine) wait_for_lock_release "$lankide_home/run/machine.lock" ;;
  esac
}

stop_daemons() {
  if command -v launchctl >/dev/null 2>&1; then
    service_domain="gui/$(id -u)"
    stop_loaded_service com.lankide.machine
    stop_loaded_service com.lankide.daemon
  fi
  stop_lock_process "$lankide_home/run/machine.lock" lankide-machine
  stop_lock_process "$lankide_home/run/runtime.lock" lankided
}

target_processes() {
  local pids pid command_line
  pids="$(ps -axo pid=,command= | awk '
    $0 ~ /lankided|lankide-machine|(^|[[:space:]])pi([[:space:]]|$)|\/pi([[:space:]]|$)|codex app-server|(^|[[:space:]])claude([[:space:]]|$)|\/claude([[:space:]]|$)/ { print $1 }
  ' | sort -u)"
  for pid in $pids; do
    [ "$pid" != "$$" ] || continue
    [ "$pid" != "$PPID" ] || continue
    command_line="$(ps -p "$pid" -o command= 2>/dev/null || true)"
    case "$command_line" in
      *lankided*|*lankide-machine*)
        case "$command_line" in
          *"$lankide_home"*) printf '%s\n' "$pid" ;;
        esac
        ;;
      *)
        if lsof -p "$pid" 2>/dev/null | grep -Fq "$workspace_root"; then
          printf '%s\n' "$pid"
        fi
        ;;
    esac
  done
}

active_lock_pids() {
  local lock pid
  for lock in "$lankide_home/run/machine.lock" "$lankide_home/run/runtime.lock"; do
    if [ -e "$lock" ]; then
      pid="$(lock_pid "$lock")"
      if kill -0 "$pid" 2>/dev/null; then
        printf '%s\n' "$pid"
      fi
    fi
  done
}

wait_for_idle() {
  local attempt openers state_openers locks processes
  for attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50
  do
    openers="$(lsof -t "$database" 2>/dev/null || true)"
    state_openers=""
    if [ "$state_existed" -eq 1 ]; then
      state_openers="$(lsof -t "$machine_state" 2>/dev/null || true)"
    fi
    locks="$(active_lock_pids)"
    processes="$(target_processes)"
    if [ -z "$openers" ] && [ -z "$state_openers" ] && [ -z "$locks" ] && [ -z "$processes" ]; then
      return 0
    fi
    sleep 0.2
  done
  printf 'Openers of the Runtime database:\n%s\n' "$openers" >&2
  printf 'Openers of the Machine state:\n%s\n' "$state_openers" >&2
  printf 'Target daemon locks:\n%s\n' "$locks" >&2
  printf 'Target processes:\n%s\n' "$processes" >&2
  die "the selected workspace or its local services are still active; refusing to reset"
}

prepare_backup() {
  local backup_parent timestamp escaped_backup state_backup
  if [ -n "$backup_arg" ]; then
    backup_parent="$backup_arg"
  else
    backup_parent="$lankide_home/backups/zero-context"
  fi
  case "$backup_parent" in
    /*) ;;
    *) die "backup directory must be absolute: $backup_parent" ;;
  esac
  if [ -e "$backup_parent" ] || [ -L "$backup_parent" ]; then
    reject_symlink "$backup_parent"
  fi
  mkdir -p "$backup_parent"
  backup_parent="$(canonical_directory "$backup_parent")"
  case "$backup_parent" in
    "$workspace_root"|"$workspace_root"/*) die "backup directory must be outside the workspace directory" ;;
  esac

  timestamp="$(date -u '+%Y%m%dT%H%M%SZ')"
  backup_root="$backup_parent/${timestamp}-${workspace_id}"
  [ ! -e "$backup_root" ] || die "backup destination already exists: $backup_root"
  mkdir -p "$backup_root"
  chmod 700 "$backup_root"

  escaped_backup="${backup_root}/runtime.db"
  escaped_backup="${escaped_backup//\'/\'\'}"
  if ! {
    printf '%s\n' '.timeout 5000'
    printf ".backup '%s'\n" "$escaped_backup"
  } | sqlite3 "$database" >/dev/null; then
    die "could not create the SQLite backup"
  fi
  [ -s "$backup_root/runtime.db" ] || die "SQLite backup is missing or empty"
  [ "$(sqlite3 "$backup_root/runtime.db" 'PRAGMA integrity_check;')" = "ok" ] || die "SQLite backup failed integrity check"

  if [ "$state_existed" -eq 1 ]; then
    state_backup="$backup_root/machine/state.json"
    mkdir -p "$(dirname "$state_backup")"
    cp -p "$machine_state" "$state_backup"
    chmod 600 "$state_backup"
  fi
}

move_path_to_backup() {
  local source="$1" destination="$2"
  if [ -e "$source" ] || [ -L "$source" ]; then
    reject_symlink "$source"
    mkdir -p "$(dirname "$destination")"
    mv "$source" "$destination" || die "could not move local history out of the workspace: $source"
    printf 'Moved local history: %s\n' "$source"
  fi
}

quarantine_agent_files() {
  local agent_id agent_home backup_agent
  for agent_id in $agent_ids; do
    agent_home="$workspace_root/agents/$agent_id"
    [ -d "$agent_home" ] || continue
    backup_agent="$backup_root/agents/$agent_id"
    move_path_to_backup "$agent_home/.lankide" "$backup_agent/.lankide"
    move_path_to_backup "$agent_home/resources" "$backup_agent/resources"
    move_path_to_backup "$agent_home/tool-results" "$backup_agent/tool-results"
    if [ -d "$backup_agent/.lankide" ]; then
      mkdir -p "$agent_home/.lankide"
      chmod 700 "$agent_home/.lankide"
    fi
    if [ -d "$backup_agent/resources" ]; then
      mkdir -p "$agent_home/resources"
      chmod 700 "$agent_home/resources"
    fi
    if [ -d "$backup_agent/tool-results" ]; then
      mkdir -p "$agent_home/tool-results"
      chmod 700 "$agent_home/tool-results"
    fi
  done
}

resource_is_kept() {
  local candidate="$1" id
  while IFS= read -r id; do
    [ -n "$id" ] || continue
    [ "$id" = "$candidate" ] && return 0
  done <<< "$keep_resource_names"
  return 1
}

quarantine_resources() {
  local source_root entry name
  source_root="$lankide_home/resources/$workspace_id"
  if [ -d "$source_root" ]; then
    shopt -s nullglob dotglob
    for entry in "$source_root"/*; do
      name="$(basename "$entry")"
      if resource_is_kept "$name"; then
        [ ! -L "$entry" ] || die "configured resource is a symbolic link: $entry"
        [ -f "$entry" ] || die "configured resource is not a regular file: $entry"
        continue
      fi
      move_path_to_backup "$entry" "$backup_root/resources/$workspace_id/$name"
    done
    shopt -u dotglob nullglob
    mkdir -p "$source_root"
    chmod 700 "$source_root"
  fi
}

clear_machine_state() {
  [ "$state_existed" -eq 1 ] || return 0
  temporary_state="$(mktemp "${machine_state}.reset.XXXXXX")"
  if ! jq \
    --arg prefix "$workspace_id/" \
    --argjson execution_ids "$target_execution_ids_json" \
      '
        def selected_execution:
          (try (.key | split(":")[0] | tonumber) catch null) as $execution_id
          | $execution_id == null or (($execution_ids | index($execution_id)) == null);
        if has("sessions") then
          .sessions |= with_entries(select((.key | startswith($prefix)) | not))
        else . end
        | if has("accepted_assignments") then
            .accepted_assignments |= with_entries(select(selected_execution))
          else . end
        | if has("pending_events") then
            .pending_events |= with_entries(select(selected_execution))
          else . end
    ' "$machine_state" > "$temporary_state"; then
    die "could not update Machine state"
  fi
  chmod 600 "$temporary_state"
  mv "$temporary_state" "$machine_state"
  temporary_state=""
}

clear_database() {
  local w resource_ids_sql
  w="$(workspace_sql)"
  resource_ids_sql="$(resource_ids_sql_value)"
  if ! sqlite3 -bail "$database" >/dev/null <<SQL
.timeout 5000
PRAGMA foreign_keys=ON;
PRAGMA secure_delete=ON;
BEGIN IMMEDIATE;
DELETE FROM execution_attempt_capabilities
 WHERE EXISTS (SELECT 1 FROM executions e WHERE e.execution_id=execution_attempt_capabilities.execution_id AND e.workspace_id=$w);
DELETE FROM execution_attempt_authorizations
 WHERE EXISTS (SELECT 1 FROM executions e WHERE e.execution_id=execution_attempt_authorizations.execution_id AND e.workspace_id=$w);
DELETE FROM execution_events
 WHERE EXISTS (SELECT 1 FROM executions e WHERE e.execution_id=execution_events.execution_id AND e.workspace_id=$w);
DELETE FROM steer_deliveries
 WHERE EXISTS (SELECT 1 FROM executions e WHERE e.execution_id=steer_deliveries.execution_id AND e.workspace_id=$w);
DELETE FROM execution_delivery_items
 WHERE EXISTS (SELECT 1 FROM execution_deliveries d JOIN executions e USING(execution_id) WHERE d.delivery_id=execution_delivery_items.delivery_id AND e.workspace_id=$w);
DELETE FROM execution_delivery_chats
 WHERE EXISTS (SELECT 1 FROM execution_deliveries d JOIN executions e USING(execution_id) WHERE d.delivery_id=execution_delivery_chats.delivery_id AND e.workspace_id=$w);
DELETE FROM execution_deliveries
 WHERE EXISTS (SELECT 1 FROM executions e WHERE e.execution_id=execution_deliveries.execution_id AND e.workspace_id=$w);
DELETE FROM tool_invocations
 WHERE EXISTS (SELECT 1 FROM executions e WHERE e.execution_id=tool_invocations.execution_id AND e.workspace_id=$w);
DELETE FROM execution_attempts
 WHERE EXISTS (SELECT 1 FROM executions e WHERE e.execution_id=execution_attempts.execution_id AND e.workspace_id=$w);
DELETE FROM message_send_attempts
 WHERE EXISTS (SELECT 1 FROM messages m JOIN chats c USING(chat_id) WHERE m.message_id=message_send_attempts.message_id AND c.workspace_id=$w)
    OR EXISTS (SELECT 1 FROM executions e WHERE e.execution_id=message_send_attempts.execution_id AND e.workspace_id=$w);
DELETE FROM executions WHERE workspace_id=$w;
DELETE FROM unread_snapshot_triggers
 WHERE EXISTS (SELECT 1 FROM unread_snapshots s WHERE s.snapshot_id=unread_snapshot_triggers.snapshot_id AND s.workspace_id=$w);
DELETE FROM unread_snapshot_messages
 WHERE EXISTS (SELECT 1 FROM unread_snapshots s WHERE s.snapshot_id=unread_snapshot_messages.snapshot_id AND s.workspace_id=$w);
DELETE FROM unread_snapshot_chats
 WHERE EXISTS (SELECT 1 FROM unread_snapshots s WHERE s.snapshot_id=unread_snapshot_chats.snapshot_id AND s.workspace_id=$w);
DELETE FROM unread_snapshots WHERE workspace_id=$w;
DELETE FROM agent_inbox_items WHERE workspace_id=$w;
DELETE FROM reactions WHERE workspace_id=$w;
DELETE FROM agent_chat_cursors WHERE workspace_id=$w;
DELETE FROM agent_chat_attention WHERE workspace_id=$w;
DELETE FROM messages
 WHERE EXISTS (SELECT 1 FROM chats c WHERE c.chat_id=messages.chat_id AND c.workspace_id=$w);
DELETE FROM chats WHERE workspace_id=$w;
DELETE FROM channels WHERE workspace_id=$w;
DELETE FROM resource_metadata WHERE workspace_id=$w AND resource_id NOT IN ($resource_ids_sql);
COMMIT;
PRAGMA wal_checkpoint(TRUNCATE);
VACUUM;
SQL
  then
    die "transactional workspace history cleanup failed"
  fi
}

check_zero() {
  local label="$1" query="$2" count
  count="$(db_count "$query")"
  [ "$count" = "0" ] || die "$label still contains $count rows"
}

verify_machine_state() {
  [ "$state_existed" -eq 1 ] || return 0
  jq -e \
    --arg prefix "$workspace_id/" \
    --argjson execution_ids "$target_execution_ids_json" \
    '
      def selected_execution:
        (try (split(":")[0] | tonumber) catch null) as $execution_id
        | $execution_id != null and (($execution_ids | index($execution_id)) != null);
      ((.sessions // {} | keys | map(select(startswith($prefix))) | length) == 0)
      and ((.accepted_assignments // {} | keys | map(select(selected_execution)) | length) == 0)
      and ((.pending_events // {} | keys | map(select(selected_execution)) | length) == 0)
    ' "$machine_state" >/dev/null || die "Machine state still contains the selected workspace"
}

verify_workspace_files() {
  local agent_id agent_home target_resources entry name
  for agent_id in $agent_ids; do
    agent_home="$workspace_root/agents/$agent_id"
    for target_resources in "$agent_home/.lankide" "$agent_home/resources" "$agent_home/tool-results"; do
      if [ -d "$target_resources" ]; then
        if find "$target_resources" -mindepth 1 -print -quit | grep -q .; then
          die "workspace-local Harness state remains under $target_resources"
        fi
      fi
    done
  done
  target_resources="$lankide_home/resources/$workspace_id"
  if [ -d "$target_resources" ]; then
    shopt -s nullglob dotglob
    for entry in "$target_resources"/*; do
      name="$(basename "$entry")"
      resource_is_kept "$name" || die "unconfigured resource remains: $entry"
    done
    shopt -u dotglob nullglob
  fi
}

verify_zero_state() {
  local w resource_ids_sql
  w="$(workspace_sql)"
  resource_ids_sql="$(resource_ids_sql_value)"
  check_zero channels "SELECT count(*) FROM channels WHERE workspace_id=$w;"
  check_zero chats "SELECT count(*) FROM chats WHERE workspace_id=$w;"
  check_zero messages "SELECT count(*) FROM messages m JOIN chats c USING(chat_id) WHERE c.workspace_id=$w;"
  check_zero reactions "SELECT count(*) FROM reactions WHERE workspace_id=$w;"
  check_zero inbox_items "SELECT count(*) FROM agent_inbox_items WHERE workspace_id=$w;"
  check_zero chat_cursors "SELECT count(*) FROM agent_chat_cursors WHERE workspace_id=$w;"
  check_zero chat_attention "SELECT count(*) FROM agent_chat_attention WHERE workspace_id=$w;"
  check_zero unread_snapshots "SELECT count(*) FROM unread_snapshots WHERE workspace_id=$w;"
  check_zero unread_snapshot_chats "SELECT count(*) FROM unread_snapshot_chats s JOIN unread_snapshots u USING(snapshot_id) WHERE u.workspace_id=$w;"
  check_zero unread_snapshot_messages "SELECT count(*) FROM unread_snapshot_messages s JOIN unread_snapshots u USING(snapshot_id) WHERE u.workspace_id=$w;"
  check_zero unread_snapshot_triggers "SELECT count(*) FROM unread_snapshot_triggers s JOIN unread_snapshots u USING(snapshot_id) WHERE u.workspace_id=$w;"
  check_zero executions "SELECT count(*) FROM executions WHERE workspace_id=$w;"
  check_zero execution_attempts "SELECT count(*) FROM execution_attempts a JOIN executions e USING(execution_id) WHERE e.workspace_id=$w;"
  check_zero execution_events "SELECT count(*) FROM execution_events x JOIN executions e USING(execution_id) WHERE e.workspace_id=$w;"
  check_zero steer_deliveries "SELECT count(*) FROM steer_deliveries d JOIN executions e USING(execution_id) WHERE e.workspace_id=$w;"
  check_zero execution_authorizations "SELECT count(*) FROM execution_attempt_authorizations a JOIN executions e USING(execution_id) WHERE e.workspace_id=$w;"
  check_zero execution_capabilities "SELECT count(*) FROM execution_attempt_capabilities a JOIN executions e USING(execution_id) WHERE e.workspace_id=$w;"
  check_zero execution_deliveries "SELECT count(*) FROM execution_deliveries d JOIN executions e USING(execution_id) WHERE e.workspace_id=$w;"
  check_zero delivery_items "SELECT count(*) FROM execution_delivery_items i JOIN execution_deliveries d USING(delivery_id) JOIN executions e USING(execution_id) WHERE e.workspace_id=$w;"
  check_zero delivery_chats "SELECT count(*) FROM execution_delivery_chats d JOIN execution_deliveries x USING(delivery_id) JOIN executions e USING(execution_id) WHERE e.workspace_id=$w;"
  check_zero message_send_attempts "SELECT count(*) FROM message_send_attempts a JOIN messages m USING(message_id) JOIN chats c USING(chat_id) WHERE c.workspace_id=$w;"
  check_zero tool_invocations "SELECT count(*) FROM tool_invocations t JOIN executions e USING(execution_id) WHERE e.workspace_id=$w;"
  check_zero removable_resources "SELECT count(*) FROM resource_metadata WHERE workspace_id=$w AND resource_id NOT IN ($resource_ids_sql);"
  [ -z "$(sqlite3 -readonly "$database" 'PRAGMA foreign_key_check;')" ] || die "PRAGMA foreign_key_check returned rows"
  verify_machine_state
  verify_workspace_files
}

remove_sqlite_sidecars() {
  local sidecar
  for sidecar in "$database-wal" "$database-shm"; do
    if [ -e "$sidecar" ] || [ -L "$sidecar" ]; then
      reject_symlink "$sidecar"
      rm -f "$sidecar" || die "could not remove SQLite runtime sidecar: $sidecar"
    fi
  done
}

service_ready() {
  local label="$1" binary="$2" lock="$3"
  launchctl print "$service_domain/$label" >/dev/null 2>&1 || return 1
  [ -f "$lankide_home/run/management.url" ] || return 1
  if [ -e "$lock" ]; then
    local pid
    pid="$(lock_pid "$lock")"
    kill -0 "$pid" 2>/dev/null || return 1
    case "$(ps -p "$pid" -o command= 2>/dev/null || true)" in
      *"$binary"*) ;;
      *) return 1 ;;
    esac
  else
    return 1
  fi
}

restart_daemons_and_verify() {
  local label plist service output attempt runtime_ready machine_ready
  [ "$lankide_home" = "$default_lankide_home" ] || die "--restart with a custom --home is unsafe; use --no-restart"
  require_command launchctl
  service_domain="gui/$(id -u)"

  for label in com.lankide.daemon com.lankide.machine; do
    plist="$user_home/Library/LaunchAgents/$label.plist"
    [ -f "$plist" ] || die "LaunchAgent plist not found: $plist"
    reject_symlink "$plist"
    service="$service_domain/$label"
    if launchctl print "$service" >/dev/null 2>&1; then
      output="$(launchctl kickstart -k "$service" 2>&1)" || die "could not restart service $service: $output"
    else
      output="$(launchctl bootstrap "$service_domain" "$plist" 2>&1)" || die "could not start service $service: $output"
    fi
  done

  runtime_ready=0
  machine_ready=0
  for attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60
  do
    if service_ready com.lankide.daemon lankided "$lankide_home/run/runtime.lock"; then
      runtime_ready=1
    fi
    if service_ready com.lankide.machine lankide-machine "$lankide_home/run/machine.lock"; then
      machine_ready=1
    fi
    if [ "$runtime_ready" -eq 1 ] && [ "$machine_ready" -eq 1 ]; then
      break
    fi
    sleep 0.2
  done
  [ "$runtime_ready" -eq 1 ] && [ "$machine_ready" -eq 1 ] || die "Runtime and Machine did not become ready after the reset"

  sleep 2
  if ! (verify_zero_state); then
    stop_daemons || true
    die "service startup reintroduced local workspace history; services were stopped again"
  fi
}

main() {
  parse_arguments "$@"
  require_command sqlite3
  require_command jq
  require_command shasum
  resolve_user_home
  resolve_lankide_home
  resolve_database
  resolve_workspace
  resolve_machine_state
  validate_database
  load_workspace_records
  validate_agent_paths
  validate_filesystem_scope
  [ "$workspace_root" != "/" ] || die "refusing to reset the filesystem root"
  print_target

  if [ "$dry_run" -eq 1 ]; then
    printf 'Dry run only: no daemon, database, or workspace file was changed.\n'
    exit 0
  fi

  require_command lsof
  require_command ps
  if [ "$restart" -eq 1 ] && [ "$lankide_home" != "$default_lankide_home" ]; then
    die "--restart with a custom --home is unsafe; use --no-restart"
  fi

  if [ "$confirmed" -eq 0 ]; then
    printf '\nThis will clear local history for workspace %s and move its Harness/session files into a recoverable backup:\n  %s\nWorkspace and Agent configuration, credentials, and normal user Harness homes will be preserved.\nAfterward Runtime and Machine will be %s.\nType RESET to continue: ' "$workspace_id" "$workspace_root" "$([ "$restart" -eq 1 ] && printf 'restarted' || printf 'left stopped')"
    local confirmation
    IFS= read -r confirmation || die "confirmation was not provided"
    [ "$confirmation" = "RESET" ] || die "reset cancelled"
  fi

  stop_daemons
  wait_for_idle

  config_before_hash="$(configuration_hash)"
  resource_config_before_hash="$(resource_configuration_hash)"
  prepare_backup
  quarantine_agent_files
  quarantine_resources
  clear_machine_state
  clear_database
  verify_zero_state
  [ "$(configuration_hash)" = "$config_before_hash" ] || die "Workspace/Agent configuration changed during reset"
  [ "$(resource_configuration_hash)" = "$resource_config_before_hash" ] || die "configured resource metadata changed during reset"
  remove_sqlite_sidecars

  if [ "$restart" -eq 1 ]; then
    restart_daemons_and_verify
  fi

  printf '\nZero-context reset completed.\n'
  printf '  Recoverable backup: %s\n' "$backup_root"
  if [ "$restart" -eq 0 ]; then
    printf '  Runtime and Machine: stopped; start them when the test is ready.\n'
  else
    printf '  Runtime and Machine: restarted with zero local workspace history.\n'
  fi
}

main "$@"
