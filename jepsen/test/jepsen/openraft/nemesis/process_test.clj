(ns jepsen.openraft.nemesis.process-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
             [nemesis :as nemesis]
             [random :as random]]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.nemesis :as openraft-nemesis]
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

(defn- failing-resume-nemesis [events error]
  (reify nemesis/Nemesis
    (setup! [this _test]
      this)
    (invoke! [_ _test op]
      (swap! events conj [(:f op) (:value op)])
      (throw error))
    (teardown! [this _test]
      (swap! events conj :teardown)
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

(deftest records-pause-recovery-history
  (let [invocations (atom [])
        delegate (recording-nemesis invocations)
        subject (process/->PauseNemesis delegate (atom nil))
        recovery (openraft-nemesis/->RecoveryNemesis)
        test {:nodes ["n1" "n2" "n3"]}
        status {:leader "n1"
                :metrics (zipmap (:nodes test) (repeat {}))}
        coverage-checker (#'process/pause-coverage-checker)
        invoke! (fn
                  ([f]
                   (nemesis/invoke! subject test {:type :info :f f}))
                  ([f value]
                   (nemesis/invoke! subject test {:type :info
                                                  :f f
                                                  :value value})))
        recover! #(nemesis/invoke! recovery
                                   test
                                   {:type :info
                                    :f :await-recovery})]
    (with-redefs [cluster/membership-status (fn [_test] status)
                  cluster/await-ready! (fn [_test] status)
                  cluster/voter-configs
                  (fn [_test _status] [(set (:nodes test))])]
      (let [leader-pause (invoke! :pause-process :leader-paused)
            _ (is (thrown-with-msg?
                   clojure.lang.ExceptionInfo
                   #"Processes are already paused"
                   (invoke! :pause-process :leader-unpaused)))
            leader-resume (invoke! :resume-process)
            leader-recovery (recover!)
            follower-pause (invoke! :pause-process :leader-unpaused)
            follower-resume (invoke! :resume-process)
            follower-recovery (recover!)
            cleanup-resume (invoke! :resume-process)
            cleanup-recovery (recover!)
            history [leader-pause
                     leader-resume
                     leader-recovery
                     follower-pause
                     follower-resume
                     follower-recovery
                     cleanup-resume
                     cleanup-recovery]
            result (checker/check coverage-checker test history {})]
        (is (= {:paused (:value leader-pause)
                :resumed (:nodes test)}
               (:value leader-resume)))
        (is (= {:paused (:value follower-pause)
                :resumed (:nodes test)}
               (:value follower-resume)))
        (is (= {:paused nil
                :resumed (:nodes test)}
               (:value cleanup-resume)))
        (is (= [[:pause (get-in leader-pause [:value :nodes])]
                [:resume (:nodes test)]
                [:pause (get-in follower-pause [:value :nodes])]
                [:resume (:nodes test)]
                [:resume (:nodes test)]]
               (mapv (juxt :f :value) @invocations)))
        (is (= {:valid? true
                :cluster-state :intact}
               (select-keys result [:valid? :cluster-state])))))))

(deftest pauses-only-reachable-voters-after-a-process-kill
  (let [invocations (atom [])
        delegate (recording-nemesis invocations)
        process-nemesis (process/->ProcessNemesis delegate (atom nil))
        pause-nemesis (process/->PauseNemesis delegate (atom nil))
        test {:nodes voters}
        metrics (atom (zipmap voters (repeat {})))
        prefer-n2 (fn [candidates]
                    (let [candidates (vec candidates)
                          preferred #{"n2"}]
                      (if (some #{preferred} candidates)
                        (cons preferred (remove #{preferred} candidates))
                        candidates)))]
    (with-redefs [cluster/membership-status (fn [_test]
                                              {:leader "n1"
                                               :metrics @metrics})
                  cluster/voter-configs (fn [_test _status]
                                          [(set voters)])
                  random/shuffle prefer-n2]
      (let [killed (nemesis/invoke! process-nemesis
                                    test
                                    {:type :info
                                     :f :kill-process
                                     :value :leader-survives})
            killed-nodes (set (get-in killed [:value :nodes]))]
        (is (= #{"n2"} killed-nodes))
        (swap! metrics #(apply dissoc % killed-nodes))
        (let [paused (nemesis/invoke! pause-nemesis
                                      test
                                      {:type :info
                                       :f :pause-process
                                       :value :leader-unpaused})
              paused-nodes (get-in paused [:value :nodes])
              delegated-pause (first (filter #(= :pause (:f %))
                                             @invocations))]
          (is (seq paused-nodes))
          (is (every? (set (keys @metrics)) paused-nodes))
          (is (= paused-nodes (:value delegated-pause))))))))

(deftest skips-a-pause-without-a-reachable-target
  (let [invocations (atom [])
        subject (process/->PauseNemesis
                 (recording-nemesis invocations)
                 (atom nil))
        test {:nodes ["n1"]}
        status {:leader "n1"
                :metrics {"n1" {}}}]
    (with-redefs [cluster/membership-status (constantly status)
                  cluster/voter-configs (fn [_test _status]
                                          [#{"n1"}])]
      (let [completion (nemesis/invoke! subject
                                        test
                                        {:type :info
                                         :f :pause-process
                                         :value :leader-unpaused})
            result (checker/check (#'process/pause-coverage-checker)
                                  test
                                  [completion]
                                  {})]
        (is (= :no-reachable-pause-target (:value completion)))
        (is (empty? @invocations))
        (is (empty? (:observed-modes result)))))))

(deftest teardown-cleanup-failure-does-not-mask-analysis
  (let [events (atom [])
        delegate (failing-resume-nemesis
                  events
                  (ex-info "unreachable" {:kind :unreachable}))
        subject (process/->PauseNemesis delegate (atom nil))
        test {:nodes ["n1" "n2" "n3"]}]
    (nemesis/teardown! subject test)
    (is (= [[:resume (:nodes test)] :teardown]
           @events))))

(deftest teardown-preserves-interruptions
  (doseq [[label error]
          [[:interrupted-exception
            (InterruptedException. "interrupted")]
           [:interrupted-io
            (java.io.InterruptedIOException. "interrupted")]
           [:closed-by-interrupt
            (java.nio.channels.ClosedByInterruptException.)]
           [:wrapped
            (ex-info "interrupted" {:kind :interrupted})]]]
    (testing (name label)
      (Thread/interrupted)
      (let [events (atom [])
            delegate (failing-resume-nemesis events error)
            subject (process/->PauseNemesis delegate (atom nil))
            test {:nodes ["n1" "n2" "n3"]}]
        (try
          (let [thrown (try
                         (nemesis/teardown! subject test)
                         nil
                         (catch Exception e
                           e))
                interrupted? (.isInterrupted (Thread/currentThread))]
            (is (identical? error thrown))
            (is interrupted?)
            (is (= [[:resume (:nodes test)] :teardown]
                   @events)))
          (finally
            (Thread/interrupted)))))))

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
        test {:nodes voters}
        check #(checker/check subject test % {})
        resumed-all {:paused nil
                     :resumed voters}
        complete-history [{:f :pause-process
                           :value {:mode :leader-unpaused}}
                          {:f :resume-process
                           :value resumed-all}
                          {:f :await-recovery
                           :value {:leader "n1"}}
                          {:f :pause-process
                           :value {:mode :leader-paused}}
                          {:f :resume-process
                           :value resumed-all}
                          {:f :await-recovery
                           :value {:leader "n2"}}]
        missing-mode-history [{:f :pause-process
                               :value {:mode :leader-unpaused}}
                              {:f :resume-process
                               :value resumed-all}
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
        paused-history (conj complete-history
                             {:f :pause-process
                              :value {:mode :leader-paused}})
        resuming-history (conj paused-history
                               {:type :info
                                :f :resume-process})
        recovered-history (into resuming-history
                                [{:f :resume-process
                                  :value resumed-all}
                                 {:f :await-recovery
                                  :value {:leader "n2"}}])
        partial-resume-history (assoc-in complete-history
                                         [4 :value :resumed]
                                         ["n1"])]
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

    (testing "a resume invocation makes a paused state indeterminate"
      (is (= {:valid? :unknown
              :cluster-state :unknown}
             (select-keys (check resuming-history)
                          [:valid? :cluster-state]))))

    (testing "a partial resume remains indeterminate after recovery"
      (is (= {:valid? :unknown
              :cluster-state :unknown}
             (select-keys (check partial-resume-history)
                          [:valid? :cluster-state]))))

    (testing "resume completion and recovery resolve an in-flight resume"
      (is (= {:valid? true
              :cluster-state :intact}
             (select-keys (check recovered-history)
                          [:valid? :cluster-state]))))))
