(ns jepsen.openraft.clock
  (:require [jepsen.control :as c]))

(def library "/usr/local/lib/libfaketime.so.1")
(def control-file "/var/lib/openraft/faketime")
(def normal-setting "+0 x1")

(def application-env
  {:LD_PRELOAD library
   :FAKETIME_TIMESTAMP_FILE control-file
   :FAKETIME_NO_CACHE "1"
   :NO_FAKE_STAT "1"
   :FAKETIME_DONT_FAKE_MONOTONIC "1"})

(def ^:private write-setting-script
  (str "set -eu; "
       "printf '%s\\n' \"$1\" > \"$2.tmp\"; "
       "mv \"$2.tmp\" \"$2\"; "
       "cat \"$2\""))

(def ^:private initialize-setting-script
  (str "set -eu; "
       "if test ! -e \"$2\"; then "
       "printf '%s\\n' \"$1\" > \"$2.tmp\"; "
       "mv \"$2.tmp\" \"$2\"; "
       "fi; "
       "cat \"$2\""))

(def ^:private process-environment-script
  (str "set -eu; "
       "pattern=$1; library=$2; control_file=$3; "
       "library_name=${library##*/}; "
       "pid=$(pgrep -u root -f --ignore-ancestors -- \"$pattern\"); "
       "test -n \"$pid\"; set -- $pid; pid=$1; "
       "grep -Fq \"/$library_name\" \"/proc/$pid/maps\"; "
       "tr '\\0' '\\n' < \"/proc/$pid/environ\" "
       "| grep -Fxq \"FAKETIME_TIMESTAMP_FILE=$control_file\"; "
       "printf 'loaded\\n'"))

(defn write-setting!
  "Atomically replaces the current node's complete faketime setting."
  [setting]
  (c/exec :bash :-c write-setting-script
          "openraft-clock-setting" setting control-file))

(defn reset-clock! []
  (write-setting! normal-setting))

(defn read-setting! []
  (c/exec :cat control-file))

(defn probe-wall-time-ms! []
  (Long/parseLong
   (c/exec (c/env application-env) :date "+%s%3N")))

(defn verify-application-clock!
  "Confirms that the application process loaded the node-local clock config."
  [process-pattern]
  (c/exec :bash :-c process-environment-script
          "openraft-clock-process" process-pattern library control-file))

(defn prepare! []
  (c/exec :mkdir :-p "/var/lib/openraft")
  (c/exec :bash :-c initialize-setting-script
          "openraft-clock-setting" normal-setting control-file))
