(ns jepsen.openraft.db
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [control :as c]
                    [core :as jepsen]
                    [db :as db]]
            [jepsen.control.util :as cu]
            [jepsen.openraft.client :as client]
            [jepsen.openraft.cluster :as cluster]))

(def binary "/usr/local/bin/raft-key-value-rocks")
(def process-name "raft-key-value-rocks")
(def data-dir "/var/lib/openraft")
(def log-dir "/var/log/openraft")
(def log-file (str log-dir "/openraft.log"))
(def pid-file (str data-dir "/openraft.pid"))

(defn- prepare-dirs! []
  (c/su
    (c/exec :mkdir :-p data-dir log-dir)))

(defn- wipe! []
  (c/su
    (c/exec :rm :-rf data-dir)
    (c/exec :rm :-f log-file)
    (c/exec :mkdir :-p data-dir log-dir)))

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
      (let [node-id (cluster/node-id test node)
            api-addr (client/api-endpoint test node)
            raft-addr (client/raft-addr test node)]
        (info node "starting OpenRaft" {:id node-id
                                        :api-addr api-addr
                                        :raft-addr raft-addr})
        (c/su
          (cu/start-daemon!
            {:chdir data-dir
             :env {:RUST_BACKTRACE "1"
                   :RUST_LOG "info"}
             :logfile log-file
             :pidfile pid-file}
            binary
            :--id node-id
            :--api-addr api-addr
            :--raft-addr raft-addr))))

    (kill! [_ _ node]
      (c/su
        (cu/stop-daemon! process-name pid-file)))))
