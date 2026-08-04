(ns jepsen.openraft.db-test
  (:require [clojure.test :refer [deftest is]]
            [jepsen.control :as c]
            [jepsen.control.util :as cu]
            [jepsen.db :as db]
            [jepsen.openraft.db :as openraft-db]))

(deftest builds-test-app-start-arguments
  (let [started-args (atom nil)
        database (openraft-db/db {})
        test {:api-port 21001
              :raft-port 22001
              :snapshot-threshold 250}]
    (with-redefs [c/exec (fn [& _args])
                  cu/start-daemon! (fn [_opts _binary & args]
                                     (reset! started-args args))]
      (db/start! database test "n1"))
    (is (= {:--id "n1"
            :--api-addr "n1:21001"
            :--raft-addr "n1:22001"
            :--snapshot-threshold 250}
           (apply hash-map @started-args)))))
