(ns jepsen.openraft.nemesis.partition-test
  (:require [clojure.set :as set]
            [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
             [nemesis :as nemesis]]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.nemesis.partition :as partition]
            [jepsen.openraft.quorum :as quorum]))

(def nodes ["n1" "n2" "n3"])

(deftest places-leader-in-requested-component
  (let [configs [(set nodes)]
        quorums (set (quorum/quorum-sets configs))]
    (doseq [mode [:leader-in-majority :leader-in-minority]]
      (testing (name mode)
        (let [[leader-side other-side]
              (#'partition/partition-components
               nodes
               configs
               "n1"
               mode)
              quorum-side (if (= mode :leader-in-majority)
                            leader-side
                            other-side)]
          (is (contains? (set leader-side) "n1"))
          (is (contains? quorums (set quorum-side)))
          (is (= (set nodes)
                 (set (concat leader-side other-side))))
          (is (empty? (set/intersection (set leader-side)
                                        (set other-side))))
          (is (seq other-side)))))))

(deftest resolves-leader-when-partition-starts
  (let [invocations (atom [])
        partitioner (reify nemesis/Nemesis
                      (setup! [this _test]
                        this)
                      (invoke! [_ _test op]
                        (swap! invocations conj op)
                        op)
                      (teardown! [this _test]
                        this))
        subject (partition/->PartitionNemesis partitioner)
        op {:type :info
            :process :nemesis
            :f :start-partition
            :value :leader-in-minority}]
    (with-redefs [cluster/await-ready! (fn [_test]
                                         {:leader "n2"})
                  cluster/voter-configs (fn [_test _status]
                                          [(set nodes)])]
      (let [result (nemesis/invoke! subject {:nodes nodes} op)
            delegated (first @invocations)]
        (is (= :start (:f delegated)))
        (is (= #{"n2"}
               (-> result :value :components first set)))
        (is (= :leader-in-minority
               (-> result :value :mode)))
        (is (= "n2"
               (-> result :value :leader)))))))

(deftest requires-both-partitions-and-an-intact-cluster
  (let [subject (#'partition/coverage-checker)
        complete-history [{:f :start-partition
                           :value {:mode :leader-in-majority}}
                          {:f :stop-partition
                           :value :network-healed}
                          {:f :start-partition
                           :value {:mode :leader-in-minority}}
                          {:f :stop-partition
                           :value :network-healed}
                          {:f :await-recovery
                           :value {:leader "n2"}}]
        missing-mode-history [{:f :start-partition
                               :value {:mode :leader-in-majority}}
                              {:f :stop-partition
                               :value :network-healed}
                              {:f :start-partition
                               :value :leader-in-minority
                               :error :partition-failed}
                              {:f :stop-partition
                               :value :network-healed}
                              {:f :await-recovery
                               :value {:leader "n2"}}]
        unrecovered-history [{:f :start-partition
                              :value {:mode :leader-in-majority}}
                             {:f :stop-partition
                              :value :network-healed}
                             {:f :start-partition
                              :value {:mode :leader-in-minority}}
                             {:f :stop-partition
                              :value :network-healed}
                             {:f :await-recovery
                              :error :timeout}]]
    (testing "both modes complete and the cluster recovers"
      (let [result (checker/check subject {} complete-history {})]
        (is (:valid? result))
        (is (= :intact (:cluster-state result)))))
    (testing "a failed partition does not count toward coverage"
      (let [result (checker/check subject {} missing-mode-history {})]
        (is (false? (:valid? result)))
        (is (= [:leader-in-minority] (:missing-modes result)))
        (is (= :intact (:cluster-state result)))))
    (testing "a confirmed heal followed by a timeout fails recovery"
      (let [result (checker/check subject {} unrecovered-history {})]
        (is (false? (:valid? result)))
        (is (empty? (:missing-modes result)))
        (is (= :recovery-pending (:cluster-state result)))))))

(deftest handles-realistic-invoke-and-completion-history
  (let [subject (#'partition/coverage-checker)
        history [{:type :info
                  :process :nemesis
                  :f :start-partition
                  :value :leader-in-majority}
                 {:type :info
                  :process :nemesis
                  :f :start-partition
                  :value {:mode :leader-in-majority
                          :leader "n1"}}
                 {:type :info
                  :process :nemesis
                  :f :stop-partition
                  :value nil}
                 {:type :info
                  :process :nemesis
                  :f :stop-partition
                  :value :network-healed}
                 {:type :info
                  :process :nemesis
                  :f :start-partition
                  :value :leader-in-minority}
                 {:type :info
                  :process :nemesis
                  :f :start-partition
                  :value {:mode :leader-in-minority
                          :leader "n1"}}
                 {:type :info
                  :process :nemesis
                  :f :stop-partition
                  :value nil}
                 {:type :info
                  :process :nemesis
                  :f :stop-partition
                  :value :network-healed}
                 {:type :info
                  :process :nemesis
                  :f :await-recovery
                  :value nil}
                 {:type :info
                  :process :nemesis
                  :f :await-recovery
                  :value {:leader "n2"}}
                 {:type :info
                  :process :nemesis
                  :f :await-recovery
                  :value nil}
                 {:type :info
                  :process :nemesis
                  :f :await-recovery
                  :value {:leader "n2"}}]
        result (checker/check subject {} history {})]
    (is (:valid? result))
    (is (= [:leader-in-majority :leader-in-minority]
           (:observed-modes result)))
    (is (= :intact (:cluster-state result)))))

(deftest reports-an-indeterminate-partition-state
  (let [subject (#'partition/coverage-checker)
        covered-history [{:f :start-partition
                          :value {:mode :leader-in-majority}}
                         {:f :stop-partition
                          :value :network-healed}
                         {:f :start-partition
                          :value {:mode :leader-in-minority}}
                         {:f :stop-partition
                          :value :network-healed}
                         {:f :await-recovery
                          :value {:leader "n2"}}]
        indeterminate-history
        (conj covered-history
              {:type :info
               :process :nemesis
               :f :start-partition
               :value :leader-in-majority
               :error "indeterminate: partition failed"
               :exception (ex-info "partition failed" {})})
        result (checker/check subject {} indeterminate-history {})]
    (is (= :unknown (:valid? result)))
    (is (= :unknown (:cluster-state result)))))

(deftest reports-an-indeterminate-heal-state
  (let [subject (#'partition/coverage-checker)
        history-before-heal [{:f :start-partition
                              :value {:mode :leader-in-majority}}
                             {:f :stop-partition
                              :value :network-healed}
                             {:f :start-partition
                              :value {:mode :leader-in-minority}}
                             {:f :stop-partition
                              :error :heal-failed}]
        indeterminate-history (conj history-before-heal
                                    {:f :await-recovery
                                     :value {:leader "n2"}})
        recovered-history (into history-before-heal
                                [{:f :stop-partition
                                  :value :network-healed}
                                 {:f :await-recovery
                                  :value {:leader "n2"}}])]
    (testing "a successful readiness check cannot prove that a failed heal completed"
      (let [result (checker/check subject {} indeterminate-history {})]
        (is (= :unknown (:valid? result)))
        (is (= :unknown (:cluster-state result)))))
    (testing "a later confirmed heal and recovery restore the intact state"
      (let [result (checker/check subject {} recovered-history {})]
        (is (:valid? result))
        (is (= :intact (:cluster-state result)))))))
