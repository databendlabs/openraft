(ns jepsen.openraft.nemesis-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.checker :as checker]
            [jepsen.openraft.db :as db]
            [jepsen.openraft.nemesis :as openraft-nemesis]
            [jepsen.openraft.nemesis.partition :as partition]
            [jepsen.openraft.nemesis.process :as process]))

(deftest cleans-up-all-faults-before-confirming-recovery
  (let [database (db/db {})
        package (openraft-nemesis/compose-packages
                 [(partition/partition-package)
                  (process/process-package database)
                  (process/pause-package database)])]
    (is (= [:stop-partition
            :restart-process
            :resume-process
            :await-recovery]
           (mapv :f (:final-generator package))))))

(deftest requires-each-composed-fault-class-to-execute
  (let [database (db/db {})
        package (openraft-nemesis/compose-packages
                 [(partition/partition-package)
                  (process/pause-package database)])
        check (fn [history]
                (checker/check (:checker package) {} history {}))
        history [{:f :start-partition
                  :value {:mode :leader-in-majority}}
                 {:f :stop-partition
                  :value :network-healed}
                 {:f :pause-process
                  :value {:mode :leader-unpaused}}
                 {:f :resume-process
                  :value {:mode :leader-unpaused}}
                 {:f :await-recovery
                  :value {:leader "n1"}}]]
    (testing "one successful mode covers each fault class in chaos"
      (let [result (check history)]
        (is (:valid? result))
        (is (true? (get-in result [:partition :fault-class-executed?])))
        (is (true? (get-in result [:pause :fault-class-executed?])))))

    (testing "a skipped fault does not count as execution"
      (let [result (check (mapv #(if (= :pause-process (:f %))
                                   (assoc % :value :no-supported-leader)
                                   %)
                                history))]
        (is (false? (:valid? result)))
        (is (false? (get-in result [:pause :fault-class-executed?])))))))
