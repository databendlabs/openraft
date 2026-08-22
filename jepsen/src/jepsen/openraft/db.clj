(ns jepsen.openraft.db
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [control :as c]
             [core :as jepsen]
             [db :as db]]
            [jepsen.control.util :as cu]
            [jepsen.openraft.client :as client]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.interruption :as interruption]))

(def binary "/usr/local/bin/openraft-jepsen-app")
(def process-name "openraft-jepsen-app")
(def data-dir "/var/lib/openraft")
(def log-dir "/var/log/openraft")
(def log-file (str log-dir "/openraft.log"))
(def default-snapshot-threshold 100)

(def ^:private process-confirm-timeout-ms 30000)
(def ^:private process-confirm-interval-ms 100)

(def ^:private process-probe-script
  (str "pids=$(pgrep -u root -f --ignore-ancestors -- \"$1\"); "
       "rc=$?; case \"$rc\" in "
       "0) ;; "
       "1) printf 'absent\\n'; exit 0 ;; "
       "*) exit \"$rc\" ;; esac; "
       "seen=0; paused=0; running=0; other=0; "
       "for pid in $pids; do "
       "stat_file=\"/proc/$pid/stat\"; "
       "if IFS= read -r stat < \"$stat_file\"; then "
       "rest=${stat##*) }; state=${rest%% *}; "
       "seen=$((seen + 1)); "
       "case \"$state\" in "
       "T|t) paused=$((paused + 1)) ;; "
       "Z|X|x|'') other=$((other + 1)) ;; "
       "*) running=$((running + 1)) ;; esac; "
       "elif [ ! -e \"/proc/$pid\" ]; then :; "
       "else printf 'unable to read %s\\n' \"$stat_file\" >&2; "
       "exit 74; fi; "
       "done; "
       "if [ \"$seen\" -eq 0 ]; then printf 'absent\\n'; "
       "elif [ \"$other\" -ne 0 ]; then printf 'mixed\\n'; "
       "elif [ \"$paused\" -eq \"$seen\" ]; then printf 'paused\\n'; "
       "elif [ \"$running\" -eq \"$seen\" ]; then printf 'running\\n'; "
       "else printf 'mixed\\n'; fi"))

(defn- process-pattern []
  (str "^" binary "([[:space:]].*)?$"))

(defn- rethrow-control! [e]
  (when (interruption/interruption? e)
    (.interrupt (Thread/currentThread)))
  (throw e))

(defn- control-exec! [& command]
  (try
    (apply c/exec command)
    (catch Exception e
      (rethrow-control! e))))

(defn- probe-process! []
  (let [result (control-exec! :bash
                              :-c
                              process-probe-script
                              "openraft-process-probe"
                              (process-pattern))]
    (case result
      "absent" :absent
      "running" :running
      "paused" :paused
      "mixed" :mixed
      (throw (ex-info "Unexpected OpenRaft process probe result"
                      {:kind :unexpected-process-probe-result
                       :result result})))))

(defn- await-process-state! [expected-state]
  (let [deadline (+ (System/nanoTime)
                    (* 1000000 process-confirm-timeout-ms))]
    (loop []
      (let [state (probe-process!)]
        (cond
          (= expected-state state)
          state

          (>= (System/nanoTime) deadline)
          (throw (ex-info "Timed out confirming OpenRaft process state"
                          {:kind :process-confirmation-timeout
                           :expected-state expected-state
                           :observed-state state
                           :timeout-ms process-confirm-timeout-ms}))

          :else
          (do
            (try
              (Thread/sleep process-confirm-interval-ms)
              (catch InterruptedException e
                (rethrow-control! e)))
            (recur)))))))

(defn- no-such-process-race? [e]
  (let [{:keys [type exit err]} (ex-data e)
        binary-name-pattern (str "(?:"
                                 (java.util.regex.Pattern/quote binary)
                                 "|"
                                 (java.util.regex.Pattern/quote process-name)
                                 ")")]
    (and (not (interruption/interruption? e))
         (= :jepsen.control/nonzero-exit type)
         (= 1 exit)
         (re-matches (re-pattern
                      (str "(?is)\\s*"
                           binary-name-pattern
                           ":\\s*no process found\\s*"))
                     (or err "")))))

(defn- signal-process! [signal confirmed-result]
  (try
    (control-exec! :killall :-s signal binary)
    confirmed-result
    (catch Exception e
      (if (no-such-process-race? e)
        :target-already-exited
        (rethrow-control! e)))))

(defn- signal-and-confirm! [signal expected-state confirmed-result]
  (if (= :absent (probe-process!))
    :target-absent
    (let [result (signal-process! signal confirmed-result)]
      (if (= :target-already-exited result)
        result
        (do
          (await-process-state! expected-state)
          result)))))

(defn- stop-process! []
  (signal-and-confirm! 9 :absent :killed))

(defn- pause-process! []
  (signal-and-confirm! "STOP" :paused :paused))

(defn- resume-process! []
  (signal-and-confirm! "CONT" :running :resumed))

(defn- start-command!
  [node-id api-addr raft-addr snapshot-threshold]
  (control-exec!
   (c/env {:RUST_BACKTRACE "1"
           :RUST_LOG "info"})
   :start-stop-daemon
   :--start
   :--oknodo
   :--background
   :--no-close
   :--exec binary
   :--chdir data-dir
   :--startas binary
   :--
   :--id node-id
   :--api-addr api-addr
   :--raft-addr raft-addr
   :--snapshot-threshold snapshot-threshold
   :>> log-file
   (c/lit "2>&1")))

(defn- start-process!
  [node-id api-addr raft-addr snapshot-threshold]
  (let [state (probe-process!)]
    (case state
      :running
      :already-running

      :absent
      (do
        ;; --oknodo makes a start race succeed without broadly suppressing exit
        ;; status 1, which may instead mean a control or permission failure.
        (start-command! node-id api-addr raft-addr snapshot-threshold)
        (await-process-state! :running)
        :start-confirmed)

      (throw (ex-info "OpenRaft process exists in an unexpected state"
                      {:kind :unexpected-existing-process-state
                       :state state})))))

(defn- prepare-dirs! []
  (c/su
   (control-exec! :mkdir :-p data-dir log-dir)))

(defn- wipe-data! []
  (c/su
   (control-exec! :rm :-rf data-dir)
   (control-exec! :mkdir :-p data-dir log-dir)))

(defn- wipe! []
  (wipe-data!)
  (c/su
   (control-exec! :rm :-f log-file)))

(defn db [_opts]
  (reify
    db/DB
    (setup! [this test node]
      (info node "setting up OpenRaft")
      (db/kill! this test node)
      (wipe!)
      (db/start! this test node)
      (cu/await-tcp-port (client/node-host node) (:api-port test client/default-api-port)
                         {:timeout 60000})
      (jepsen/synchronize test)
      (when (= node (jepsen/primary test))
        (cluster/bootstrap! test)))

    (teardown! [this test node]
      (info node "tearing down OpenRaft")
      (db/kill! this test node)
      (wipe!))

    db/LogFiles
    (log-files [_ _ _]
      {log-file "openraft.log"})

    db/Kill
    (start! [_ test node]
      (prepare-dirs!)
      (let [node-id (client/node-host node)
            api-addr (client/api-endpoint test node)
            raft-addr (client/raft-addr test node)]
        (info node "starting OpenRaft" {:id node-id
                                        :api-addr api-addr
                                        :raft-addr raft-addr})
        (c/su
         (start-process! node-id
                         api-addr
                         raft-addr
                         (:snapshot-threshold test)))))

    (kill! [_ _ _node]
      (c/su
       (stop-process!)))

    db/Pause
    (pause! [_ _ _node]
      (c/su
       (pause-process!)))

    (resume! [_ _ _node]
      (c/su
       (resume-process!)))))

(defn- start-empty-node-on-current-node! [database test node]
  (info node "starting an empty OpenRaft node")
  (db/kill! database test node)
  (wipe-data!)
  (db/start! database test node))

(defn start-empty-node-without-wait! [database test node]
  (c/on-nodes
   test
   [node]
   (fn [test node]
     (start-empty-node-on-current-node! database test node))))

(defn start-empty-node! [database test node]
  (c/on-nodes
   test
   [node]
   (fn [test node]
     (start-empty-node-on-current-node! database test node)
     (cu/await-tcp-port
      (client/node-host node)
      (:api-port test client/default-api-port)
      {:timeout 60000}))))

(defn stop-and-wipe-node! [database test node]
  (c/on-nodes
   test
   [node]
   (fn [test node]
     (info node "stopping and wiping the removed OpenRaft node")
     (db/kill! database test node)
     (wipe-data!))))
