(ns jepsen.openraft.nemesis-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.checker :as checker]
            [jepsen.generator :as gen]
            [jepsen.generator.test :as gen-test]
            [jepsen.openraft.db :as db]
            [jepsen.openraft.nemesis :as openraft-nemesis]
            [jepsen.openraft.nemesis.membership :as membership]
            [jepsen.openraft.nemesis.partition :as partition]
            [jepsen.openraft.nemesis.process :as process]))

(def test-config
  {:nodes ["n1" "n2" "n3" "n4" "n5"]})

(deftest schedules-and-retries-skipped-faults
  (doseq [skip-result [:no-supported-leader
                       :no-reachable-pause-target]]
    (testing (name skip-result)
      (let [faults (#'openraft-nemesis/interval-schedule
                    10
                    3
                    (gen/cycle [{:type :info :f :first}
                                {:type :info :f :second}]))
            generator (gen/shortest-any
                       (gen/nemesis (gen/time-limit 40 faults))
                       (gen/clients
                        (gen/stagger 0.1 (repeat {:f :read}))))
            completion-latency (gen/secs->nanos 2)
            first-attempt? (atom true)
            history (gen-test/simulate
                     generator
                     (fn [_ operation]
                       (let [skip? (and (= :first (:f operation))
                                        (compare-and-set! first-attempt?
                                                          true
                                                          false))]
                         (cond-> (assoc operation :type :ok)
                           skip? (assoc :value skip-result)
                           skip? (update :time + completion-latency)))))
            fault-ops (->> history
                           (filter #(and (= :nemesis (:process %))
                                         (= :info (:type %))))
                           (take 3))
            [_ retry next-fault] fault-ops]
        (is (= [:first :first :second] (mapv :f fault-ops)))
        (is (= (gen/secs->nanos 3) (:time (first fault-ops))))
        (is (<= (gen/secs->nanos 5.5)
                (:time retry)
                (gen/secs->nanos 6.5)))
        (is (<= (+ (:time retry) (gen/secs->nanos 5))
                (:time next-fault)
                (+ (:time retry) (gen/secs->nanos 15))))))))

(deftest cleans-up-all-faults-before-confirming-recovery
  (let [database (db/db {})
        package (openraft-nemesis/compose-packages
                 [(membership/membership-package
                   database
                   test-config)
                  (partition/partition-package)
                  (process/process-package database)
                  (process/pause-package database)])]
    (is (= [:stop-partition
            :restart-process
            :resume-process
            :restore-membership
            :await-recovery]
           (mapv :f (:final-generator package))))))

(deftest requires-each-composed-fault-class-to-execute
  (let [database (db/db {})
        all-voters (set (:nodes test-config))
        fewer-voters (disj all-voters "n5")
        package (openraft-nemesis/compose-packages
                 [(partition/partition-package)
                  (process/pause-package database)
                  (membership/membership-package database test-config)])
        check (fn [history]
                (checker/check (:checker package)
                               test-config
                               history
                               {}))
        history [{:f :start-partition
                  :value {:mode :leader-in-majority}}
                 {:f :stop-partition
                  :value :network-healed}
                 {:f :pause-process
                  :value {:mode :leader-unpaused}}
                 {:f :resume-process
                  :value {:mode :leader-unpaused}}
                 {:f :shrink
                  :value {:change :shrink
                          :before all-voters
                          :after fewer-voters}}
                 {:f :restore-membership
                  :value {:voters all-voters}}
                 {:f :await-recovery
                  :value {:leader "n1"}}]]
    (testing "one successful mode covers each fault class in chaos"
      (let [result (check history)]
        (is (:valid? result))
        (is (true? (get-in result [:partition :fault-class-executed?])))
        (is (true? (get-in result [:pause :fault-class-executed?])))
        (is (true? (get-in result
                           [:membership :fault-class-executed?])))))

    (testing "a skipped fault does not count as execution"
      (let [result (check (mapv #(if (= :pause-process (:f %))
                                   (assoc % :value :no-supported-leader)
                                   %)
                                history))]
        (is (false? (:valid? result)))
        (is (false? (get-in result [:pause :fault-class-executed?])))))))
