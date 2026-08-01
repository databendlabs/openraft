(ns jepsen.openraft.nemesis.process-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
             [nemesis :as nemesis]]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.nemesis.process :as process]
            [jepsen.openraft.quorum :as quorum]))

(def voters ["n1" "n2" "n3" "n4" "n5"])

(defn- recording-nemesis [invocations]
  (reify nemesis/Nemesis
    (setup! [this _test]
      this)
    (invoke! [_ _test op]
      (swap! invocations conj op)
      op)
    (teardown! [this _test]
      this)))

(deftest selects-fault-set-for-leader-mode
  (let [configs [(set voters)]
        fault-sets (set (quorum/fault-sets configs))]
    (doseq [mode [:leader-killed
                  :leader-survives
                  :leader-paused
                  :leader-unpaused]]
      (testing (name mode)
        (let [targets (#'process/process-targets
                       voters
                       configs
                       "n1"
                       mode)]
          (is (contains? fault-sets (set targets)))
          (is (= (boolean (#{:leader-killed :leader-paused} mode))
                 (contains? (set targets) "n1"))))))))

(deftest restarts-the-processes-that-were-killed
  (let [invocations (atom [])
        delegate (recording-nemesis invocations)
        subject (process/->ProcessNemesis delegate (atom nil))
        test {:nodes ["n1" "n2" "n3"]}
        status {:leader "n1"}]
    (with-redefs [cluster/membership-status (fn [_test] status)
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

(deftest resumes-all-processes-after-a-pause
  (let [invocations (atom [])
        delegate (recording-nemesis invocations)
        paused {:nodes ["n1"]}
        subject (process/->PauseNemesis delegate (atom paused))
        test {:nodes ["n1" "n2" "n3"]}]
    (nemesis/invoke! subject
                     test
                     {:type :info
                      :f :resume-process})
    (is (= [[:resume ["n1" "n2" "n3"]]]
           (mapv (juxt :f :value) @invocations)))))

(deftest skips-disruptions-without-a-supported-leader
  (let [invocations (atom [])
        delegate (recording-nemesis invocations)
        test {:nodes ["n1" "n2" "n3"]}
        operations [[(process/->ProcessNemesis delegate (atom nil))
                     {:type :info
                      :f :kill-process
                      :value :leader-killed}]
                    [(process/->PauseNemesis delegate (atom nil))
                     {:type :info
                      :f :pause-process
                      :value :leader-paused}]]]
    (with-redefs [cluster/membership-status (constantly nil)]
      (doseq [[subject op] operations]
        (is (= :no-supported-leader
               (:value (nemesis/invoke! subject test op))))))
    (is (empty? @invocations))))

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
                               :value {:mode :leader-survives}}
                              {:f :kill-process
                               :value :no-supported-leader}]
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

(deftest checks-pause-coverage-and-recovery-state
  (let [subject (#'process/pause-coverage-checker)
        check #(checker/check subject {} % {})
        complete-history [{:f :pause-process
                           :value {:mode :leader-unpaused}}
                          {:f :resume-process
                           :value :all-processes-resumed}
                          {:f :await-recovery
                           :value {:leader "n1"}}
                          {:f :pause-process
                           :value {:mode :leader-paused}}
                          {:f :resume-process
                           :value :all-processes-resumed}
                          {:f :await-recovery
                           :value {:leader "n2"}}]
        missing-mode-history [{:f :pause-process
                               :value {:mode :leader-unpaused}}
                              {:f :resume-process
                               :value :all-processes-resumed}
                              {:f :await-recovery
                               :value {:leader "n1"}}
                              {:f :pause-process
                               :value :no-supported-leader}]
        unrecovered-history (-> complete-history
                                pop
                                (conj {:f :await-recovery
                                       :error :timeout}))
        indeterminate-history (conj complete-history
                                    {:f :pause-process
                                     :value :leader-paused
                                     :error :pause-failed})
        resuming-history (conj indeterminate-history
                               {:type :invoke
                                :f :resume-process})
        recovered-history (into resuming-history
                                [{:f :resume-process
                                  :value :all-processes-resumed}
                                 {:f :await-recovery
                                  :value {:leader "n2"}}])]
    (testing "both pause modes complete and recover"
      (is (= {:valid? true
              :cluster-state :intact}
             (select-keys (check complete-history)
                          [:valid? :cluster-state]))))

    (testing "a pause mode is missing"
      (is (= {:valid? false
              :missing-modes [:leader-paused]
              :cluster-state :intact}
             (select-keys (check missing-mode-history)
                          [:valid? :missing-modes :cluster-state]))))

    (testing "the final resume is not followed by recovery"
      (is (= {:valid? false
              :missing-modes []
              :cluster-state :recovery-pending}
             (select-keys (check unrecovered-history)
                          [:valid? :missing-modes :cluster-state]))))

    (testing "a pause result is indeterminate"
      (is (= {:valid? :unknown
              :cluster-state :unknown}
             (select-keys (check indeterminate-history)
                          [:valid? :cluster-state]))))

    (testing "a resume invocation does not resolve an indeterminate pause"
      (is (= {:valid? :unknown
              :cluster-state :unknown}
             (select-keys (check resuming-history)
                          [:valid? :cluster-state]))))

    (testing "global resume and recovery resolve an indeterminate pause"
      (is (= {:valid? true
              :cluster-state :intact}
             (select-keys (check recovered-history)
                          [:valid? :cluster-state]))))))
