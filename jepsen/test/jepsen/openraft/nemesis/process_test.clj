(ns jepsen.openraft.nemesis.process-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
             [nemesis :as nemesis]]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.nemesis.process :as process]
            [jepsen.openraft.quorum :as quorum]))

(def voters ["n1" "n2" "n3" "n4" "n5"])

(deftest selects-fault-set-for-leader-mode
  (let [configs [(set voters)]
        fault-sets (set (quorum/fault-sets configs))]
    (doseq [mode [:leader-killed :leader-survives]]
      (testing (name mode)
        (let [targets (#'process/process-targets
                       voters
                       configs
                       "n1"
                       mode)]
          (is (contains? fault-sets (set targets)))
          (is (= (= mode :leader-killed)
                 (contains? (set targets) "n1"))))))))

(deftest restarts-the-processes-that-were-killed
  (let [invocations (atom [])
        delegate (reify nemesis/Nemesis
                   (setup! [this _test]
                     this)
                   (invoke! [_ _test op]
                     (swap! invocations conj op)
                     op)
                   (teardown! [this _test]
                     this))
        subject (process/->ProcessNemesis delegate (atom nil))
        test {:nodes ["n1" "n2" "n3"]}
        status {:leader "n1"}]
    (with-redefs [cluster/await-ready! (fn [_test] status)
                  cluster/voter-configs
                  (fn [_test _status] [(set (:nodes test))])]
      (let [killed (nemesis/invoke!
                    subject
                    test
                    {:type :info
                     :f :kill-process
                     :value :leader-killed})
            restarted (nemesis/invoke!
                       subject
                       test
                       {:type :info
                        :f :restart-process})]
        (is (= ["n1"] (get-in killed [:value :nodes])))
        (is (= ["n1"] (get-in restarted [:value :nodes])))
        (is (= [[:kill ["n1"]]
                [:start ["n1"]]]
               (mapv (juxt :f :value) @invocations)))))))

(deftest requires-both-process-modes-and-an-intact-cluster
  (let [subject (#'process/coverage-checker)
        complete-history [{:f :kill-process
                           :value {:mode :leader-survives}}
                          {:f :await-recovery
                           :value {:leader "n1"}}
                          {:f :kill-process
                           :value {:mode :leader-killed}}
                          {:f :await-recovery
                           :value {:leader "n2"}}]
        missing-mode-history [{:f :kill-process
                               :value {:mode :leader-survives}}]
        unrecovered-history [{:f :kill-process
                              :value {:mode :leader-survives}}
                             {:f :await-recovery
                              :value {:leader "n1"}}
                             {:f :kill-process
                              :value {:mode :leader-killed}}
                             {:f :await-recovery
                              :error :timeout}]]
    (let [result (checker/check subject {} complete-history {})]
      (is (:valid? result))
      (is (= :intact (:cluster-state result))))
    (let [result (checker/check subject {} missing-mode-history {})]
      (is (false? (:valid? result)))
      (is (= [:leader-killed] (:missing-modes result)))
      (is (= :degraded (:cluster-state result))))
    (let [result (checker/check subject {} unrecovered-history {})]
      (is (false? (:valid? result)))
      (is (empty? (:missing-modes result)))
      (is (= :degraded (:cluster-state result))))))

(deftest reports-an-indeterminate-process-state
  (let [subject (#'process/coverage-checker)
        covered-history [{:f :kill-process
                          :value {:mode :leader-survives}}
                         {:f :await-recovery
                          :value {:leader "n1"}}
                         {:f :kill-process
                          :value {:mode :leader-killed}}
                         {:f :await-recovery
                          :value {:leader "n2"}}]
        indeterminate-history (conj covered-history
                                    {:f :kill-process
                                     :value :leader-killed
                                     :error :kill-failed})
        recovered-history (conj indeterminate-history
                                {:f :await-recovery
                                 :value {:leader "n2"}})]
    (let [result (checker/check subject {} indeterminate-history {})]
      (is (= :unknown (:valid? result)))
      (is (= :unknown (:cluster-state result))))
    (let [result (checker/check subject {} recovered-history {})]
      (is (:valid? result))
      (is (= :intact (:cluster-state result))))))
