(ns jepsen.openraft.cli
  (:gen-class)
  (:require [jepsen [checker :as checker]
                    [cli :as cli]
                    [nemesis :as nemesis]
                    [tests :as tests]]
            [jepsen.openraft [db :as openraft-db]
                             [workload :as workload]]))

(def cli-opts
  [[nil "--api-port PORT" "OpenRaft application HTTP port."
    :default 21001
    :parse-fn parse-long]

   [nil "--raft-port PORT" "OpenRaft internal Raft RPC port."
    :default 22001
    :parse-fn parse-long]])

(defn openraft-test [opts]
  (let [workload (workload/workload opts)]
    (merge tests/noop-test
           opts
           workload
           {:name "openraft linearizable register"
            :db (openraft-db/db opts)
            :nemesis nemesis/noop
            :checker (checker/compose
                        {:stats (checker/stats)
                         :workload (:checker workload)})})))

(defn -main [& args]
  (cli/run! (cli/single-test-cmd {:test-fn openraft-test
                                  :opt-spec cli-opts
                                  :usage (cli/test-usage)})
            args))
