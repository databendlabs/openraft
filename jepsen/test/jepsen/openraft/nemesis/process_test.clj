(ns jepsen.openraft.nemesis.process-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
             [db :as db]
             [nemesis :as nemesis]
             [random :as random]]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.harness :as harness]
            [jepsen.openraft.nemesis :as openraft-nemesis]
            [jepsen.openraft.nemesis.process :as process]
            [jepsen.openraft.quorum :as quorum]
            [jepsen.openraft.worker :as worker]))

(def voters ["n1" "n2" "n3" "n4" "n5"])

(defn- installed [details]
  (assoc details :status :installed))

(defn- skipped [reason details]
  (assoc details :status :skipped :reason reason))

(defn- indeterminate [reason details]
  (assoc details :status :indeterminate :reason reason))

(defn- delegate-completion [op]
  (case (:f op)
    :kill (assoc op :value (zipmap (:value op) (repeat :killed)))
    :start (assoc op :value (zipmap (:value op)
                                    (repeat :start-confirmed)))
    :pause (assoc op :value (zipmap (:value op) (repeat :paused)))
    :resume (assoc op :value (zipmap (:value op) (repeat :resumed)))
    op))

(defn- recording-nemesis [invocations]
  (reify nemesis/Nemesis
    (setup! [this _test]
      this)
    (invoke! [_ _test op]
      (swap! invocations conj op)
      (delegate-completion op))
    (teardown! [this _test]
      this)))

(defn- failing-nemesis [invocations failing-f error]
  (reify nemesis/Nemesis
    (setup! [this _test]
      this)
    (invoke! [_ _test op]
      (swap! invocations conj op)
      (when (= failing-f (:f op))
        (throw error))
      (delegate-completion op))
    (teardown! [this _test]
      this)))

(defn- failing-resume-nemesis
  ([events resume-error]
   (failing-resume-nemesis events resume-error nil))
  ([events resume-error teardown-error]
   (reify nemesis/Nemesis
     (setup! [this _test]
       this)
     (invoke! [_ _test op]
       (swap! events conj [(:f op) (:value op)])
       (throw resume-error))
     (teardown! [this _test]
       (swap! events conj :teardown)
       (when teardown-error
         (throw teardown-error))
       this))))

(defn- failing-teardown-nemesis [events error]
  (reify nemesis/Nemesis
    (setup! [this _test]
      this)
    (invoke! [_ _test op]
      (swap! events conj [(:f op) (:value op)])
      (delegate-completion op))
    (teardown! [_ _test]
      (swap! events conj :teardown)
      (throw error))))

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
        (is (= :installed (get-in killed [:value :status])))
        (is (= :installed (get-in restarted [:value :status])))
        (is (= {"n1" :killed}
               (get-in killed [:value :stop-results])))
        (is (= {"n1" :start-confirmed}
               (get-in restarted [:value :start-results])))
        (is (= [[:kill ["n1"]]
                [:start ["n1"]]]
               (mapv (juxt :f :value) @invocations)))))))

(deftest derives-process-outcomes-from-confirmed-node-results
  (let [test {:nodes ["n1" "n2" "n3"]}
        status {:leader "n1"}
        invoke-with-results
        (fn [results]
          (let [delegate (reify nemesis/Nemesis
                           (setup! [this _test] this)
                           (invoke! [_ _test op]
                             (assoc op :value results))
                           (teardown! [this _test] this))
                subject (process/->ProcessNemesis delegate (atom nil))]
            (with-redefs [cluster/membership-status (constantly status)
                          cluster/voter-configs
                          (fn [_test _status] [(set (:nodes test))])]
              (nemesis/invoke! subject
                               test
                               {:type :info
                                :f :kill-process
                                :value :leader-killed}))))]
    (testing "every pre-probe absence skips the kill"
      (let [value (:value (invoke-with-results {"n1" :target-absent}))]
        (is (= :skipped (:status value)))
        (is (= :target-absent (:reason value)))))

    (testing "an explicit exit race skips the kill"
      (let [value (:value
                   (invoke-with-results {"n1" :target-already-exited}))]
        (is (= :skipped (:status value)))
        (is (= :target-already-exited (:reason value)))))

    (testing "any confirmed kill installs a multi-node disruption"
      (let [five-node-test {:nodes voters}
            results {"n2" :killed
                     "n3" :target-absent}
            delegate (reify nemesis/Nemesis
                       (setup! [this _test] this)
                       (invoke! [_ _test op]
                         (assoc op :value results))
                       (teardown! [this _test] this))
            subject (process/->ProcessNemesis delegate (atom nil))
            prefer-targets (fn [candidates]
                             (cons #{"n2" "n3"}
                                   (remove #{#{"n2" "n3"}} candidates)))]
        (with-redefs [cluster/membership-status
                      (constantly {:leader "n1"})
                      cluster/voter-configs
                      (fn [_test _status] [(set voters)])
                      random/shuffle prefer-targets]
          (let [value (:value
                       (nemesis/invoke! subject
                                        five-node-test
                                        {:type :info
                                         :f :kill-process
                                         :value :leader-survives}))]
            (is (= :installed (:status value)))
            (is (= results (:stop-results value)))))))))

(deftest derives-pause-outcomes-from-confirmed-node-results
  (let [test {:nodes ["n1" "n2" "n3"]}
        status {:leader "n1"
                :metrics (zipmap (:nodes test) (repeat {}))}
        invoke-with-result
        (fn [result]
          (let [delegate (reify nemesis/Nemesis
                           (setup! [this _test] this)
                           (invoke! [_ _test op]
                             (assoc op
                                    :value (zipmap (:value op)
                                                   (repeat result))))
                           (teardown! [this _test] this))
                subject (process/->PauseNemesis delegate (atom nil))]
            (with-redefs [cluster/membership-status (constantly status)
                          cluster/voter-configs
                          (fn [_test _status] [(set (:nodes test))])]
              (nemesis/invoke! subject
                               test
                               {:type :info
                                :f :pause-process
                                :value :leader-paused}))))]
    (testing "every pre-probe absence skips the pause"
      (let [value (:value (invoke-with-result :target-absent))]
        (is (= :skipped (:status value)))
        (is (= :target-absent (:reason value)))
        (is (= {"n1" :target-absent} (:pause-results value)))))

    (testing "an explicit exit race skips the pause"
      (let [value (:value (invoke-with-result :target-already-exited))]
        (is (= :skipped (:status value)))
        (is (= :target-already-exited (:reason value)))
        (is (= {"n1" :target-already-exited}
               (:pause-results value)))))

    (testing "a confirmed pause installs the disruption"
      (let [value (:value (invoke-with-result :paused))]
        (is (= :installed (:status value)))
        (is (= {"n1" :paused} (:pause-results value)))))))

(deftest derives-resume-outcomes-from-confirmed-node-results
  (let [test {:nodes ["n1" "n2" "n3"]}
        disruption {:mode :leader-paused
                    :leader "n1"
                    :nodes ["n1"]
                    :voter-configs [(set (:nodes test))]
                    :survivors ["n2" "n3"]
                    :pause-results {"n1" :paused}}
        invoke-with-results
        (fn [results]
          (let [delegate (reify nemesis/Nemesis
                           (setup! [this _test] this)
                           (invoke! [_ _test op]
                             (assoc op :value results))
                           (teardown! [this _test] this))
                active (atom disruption)
                subject (process/->PauseNemesis delegate active)
                completion (nemesis/invoke! subject
                                            test
                                            {:type :info
                                             :f :resume-process})]
            [completion @active]))]
    (testing "every absent target skips the resume"
      (let [results (zipmap (:nodes test) (repeat :target-absent))
            [completion active] (invoke-with-results results)
            value (:value completion)]
        (is (= :skipped (:status value)))
        (is (= :target-absent (:reason value)))
        (is (= results (:resume-results value)))
        (is (nil? active))))

    (testing "an explicit exit race skips the resume"
      (let [results {"n1" :target-absent
                     "n2" :target-already-exited
                     "n3" :target-absent}
            [completion active] (invoke-with-results results)
            value (:value completion)]
        (is (= :skipped (:status value)))
        (is (= :target-already-exited (:reason value)))
        (is (= results (:resume-results value)))
        (is (nil? active))))

    (testing "any confirmed resume installs multi-node recovery"
      (let [results {"n1" :resumed
                     "n2" :target-absent
                     "n3" :target-already-exited}
            [completion active] (invoke-with-results results)
            value (:value completion)]
        (is (= :installed (:status value)))
        (is (= results (:resume-results value)))
        (is (= disruption (:paused value)))
        (is (nil? active))))))

(deftest rejects-malformed-process-control-results-and-retains-recovery-state
  (let [test {:nodes ["n1" "n2" "n3"]}
        invocations (atom 0)
        delegate (reify nemesis/Nemesis
                   (setup! [this _test] this)
                   (invoke! [_ _test op]
                     (swap! invocations inc)
                     (assoc op :value {}))
                   (teardown! [this _test] this))
        active (atom nil)
        subject (process/->ProcessNemesis delegate active)]
    (with-redefs [cluster/membership-status (constantly {:leader "n1"})
                  cluster/voter-configs
                  (fn [_test _status] [(set (:nodes test))])]
      (let [error (try
                    (nemesis/invoke! subject
                                     test
                                     {:type :info
                                      :f :kill-process
                                      :value :leader-killed})
                    nil
                    (catch Exception e
                      e))]
        (is (= :unexpected-process-control-result
               (:kind (ex-data error))))
        (is (= ["n1"] (:nodes @active))))

      (let [error (try
                    (nemesis/invoke! subject
                                     test
                                     {:type :info
                                      :f :restart-process})
                    nil
                    (catch Exception e
                      e))]
        (is (= :unexpected-process-control-result
               (:kind (ex-data error))))
        (is (= ["n1"] (:nodes @active)))
        (is (= 2 @invocations))))))

(deftest rejects-malformed-pause-control-results-and-retains-recovery-state
  (let [test {:nodes ["n1" "n2" "n3"]}
        status {:leader "n1"
                :metrics (zipmap (:nodes test) (repeat {}))}
        delegate (reify nemesis/Nemesis
                   (setup! [this _test] this)
                   (invoke! [_ _test op]
                     (assoc op :value {}))
                   (teardown! [this _test] this))
        active (atom nil)
        subject (process/->PauseNemesis delegate active)]
    (with-redefs [cluster/membership-status (constantly status)
                  cluster/voter-configs
                  (fn [_test _status] [(set (:nodes test))])]
      (let [error (try
                    (nemesis/invoke! subject
                                     test
                                     {:type :info
                                      :f :pause-process
                                      :value :leader-paused})
                    nil
                    (catch Exception e
                      e))]
        (is (= :unexpected-process-control-result
               (:kind (ex-data error))))
        (is (= ["n1"] (:nodes @active))))

      (let [active-before-resume @active
            error (try
                    (nemesis/invoke! subject
                                     test
                                     {:type :info
                                      :f :resume-process})
                    nil
                    (catch Exception e
                      e))]
        (is (= :unexpected-process-control-result
               (:kind (ex-data error))))
        (is (= active-before-resume @active))))))

(deftest multi-node-control-restores-the-receiving-thread-interrupt-flag
  (let [test {:nodes voters
              :sessions (zipmap voters (repeat :unused-session))}
        planned #{"n2" "n3"}
        disruption {:mode :leader-unpaused
                    :leader "n1"
                    :nodes ["n2" "n3"]
                    :voter-configs [(set voters)]
                    :survivors ["n1" "n4" "n5"]
                    :pause-results {"n2" :paused "n3" :paused}}
        prefer-planned (fn [candidates]
                         (cons planned (remove #{planned} candidates)))
        cases
        [[:kill
          #(InterruptedException. "interrupted")
          process/process-nemesis
          nil
          {:type :info :f :kill-process :value :leader-survives}]
         [:start
          #(java.io.InterruptedIOException. "interrupted")
          process/process-nemesis
          :killed
          {:type :info :f :restart-process}]
         [:pause
          #(java.nio.channels.ClosedByInterruptException.)
          process/pause-nemesis
          nil
          {:type :info :f :pause-process :value :leader-unpaused}]
         [:resume
          #(ex-info "interrupted" {:kind :interrupted})
          process/pause-nemesis
          :paused
          {:type :info :f :resume-process}]]]
    (doseq [[label make-error make-subject active-key op] cases]
      (testing (name label)
        (Thread/interrupted)
        (try
          (let [error (make-error)
                database (reify
                           db/Process
                           (kill! [_ _test _node]
                             (throw error))
                           (start! [_ _test _node]
                             (throw error))

                           db/Pause
                           (pause! [_ _test _node]
                             (throw error))
                           (resume! [_ _test _node]
                             (throw error)))
                subject (make-subject database)
                _ (when active-key
                    (reset! (get subject active-key) disruption))
                [thrown interrupted?]
                (with-redefs [cluster/membership-status
                              (constantly
                               {:leader "n1"
                                :metrics (zipmap voters (repeat {}))})
                              cluster/voter-configs
                              (fn [_test _status] [(set voters)])
                              random/shuffle prefer-planned]
                  (try
                    (nemesis/invoke! subject test op)
                    [nil (.isInterrupted (Thread/currentThread))]
                    (catch Exception e
                      [e (.isInterrupted (Thread/currentThread))])))]
            (is (identical? error thrown))
            (is interrupted?))
          (finally
            (Thread/interrupted)))))))

(deftest restarts-all-planned-processes-after-a-kill-error
  (let [invocations (atom [])
        delegate (failing-nemesis invocations
                                  :kill
                                  (ex-info "kill failed" {}))
        subject (process/->ProcessNemesis delegate (atom nil))
        test {:nodes voters}
        planned #{"n2" "n3"}
        prefer-planned (fn [candidates]
                         (cons planned (remove #{planned} candidates)))]
    (with-redefs [cluster/membership-status (constantly {:leader "n1"})
                  cluster/voter-configs (fn [_test _status]
                                          [(set voters)])
                  random/shuffle prefer-planned]
      (is (thrown-with-msg?
           clojure.lang.ExceptionInfo
           #"kill failed"
           (nemesis/invoke! subject
                            test
                            {:type :info
                             :f :kill-process
                             :value :leader-survives})))
      (let [restarted (nemesis/invoke! subject
                                       test
                                       {:type :info :f :restart-process})]
        (is (= ["n2" "n3"] (get-in restarted [:value :nodes])))
        (is (= [[:kill ["n2" "n3"]]
                [:start ["n2" "n3"]]]
               (mapv (juxt :f :value) @invocations)))))))

(deftest skips-a-pause-after-a-pause-error
  (let [invocations (atom [])
        delegate (failing-nemesis invocations
                                  :pause
                                  (ex-info "pause failed" {}))
        subject (process/->PauseNemesis delegate (atom nil))
        test {:nodes ["n1" "n2" "n3"]}
        status {:leader "n1"
                :metrics (zipmap (:nodes test) (repeat {}))}]
    (with-redefs [cluster/membership-status (fn [_test] status)
                  cluster/voter-configs (fn [_test _status]
                                          [(set (:nodes test))])]
      (is (thrown-with-msg?
           clojure.lang.ExceptionInfo
           #"pause failed"
           (nemesis/invoke! subject test {:type :info
                                          :f :pause-process
                                          :value :leader-paused})))
      (let [skipped-value
            (:value (nemesis/invoke! subject test {:type :info
                                                   :f :pause-process
                                                   :value :leader-unpaused}))]
        (is (= :skipped (:status skipped-value)))
        (is (= :processes-already-paused (:reason skipped-value))))
      (is (= [[:pause ["n1"]]]
             (mapv (juxt :f :value) @invocations)))
      (let [resumed (nemesis/invoke! subject
                                     test
                                     {:type :info :f :resume-process})]
        (is (= {:mode :leader-paused
                :leader "n1"
                :nodes ["n1"]
                :voter-configs [#{"n1" "n2" "n3"}]
                :survivors ["n2" "n3"]}
               (get-in resumed [:value :paused])))))))

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
            duplicate-pause (invoke! :pause-process :leader-unpaused)
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
        (is (= :skipped (get-in duplicate-pause [:value :status])))
        (is (= :processes-already-paused
               (get-in duplicate-pause [:value :reason])))
        (is (= {:paused (dissoc (:value leader-pause) :status)
                :resumed (:nodes test)
                :resume-results (zipmap (:nodes test) (repeat :resumed))
                :status :installed}
               (:value leader-resume)))
        (is (= {:paused (dissoc (:value follower-pause) :status)
                :resumed (:nodes test)
                :resume-results (zipmap (:nodes test) (repeat :resumed))
                :status :installed}
               (:value follower-resume)))
        (is (= {:paused nil
                :resumed (:nodes test)
                :resume-results (zipmap (:nodes test) (repeat :resumed))
                :status :installed}
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
        (is (= :skipped (get-in completion [:value :status])))
        (is (= :no-reachable-pause-target
               (get-in completion [:value :reason])))
        (is (empty? @invocations))
        (is (empty? (:observed-modes result)))))))

(deftest teardown-cleanup-failure-records-a-harness-failure
  (let [failure-state (harness/failure-state)
        events (atom [])
        error (ex-info "unreachable" {:kind :unreachable})
        delegate (failing-resume-nemesis
                  events
                  error)
        subject (worker/wrap-nemesis-teardown
                 failure-state
                 :pause
                 (process/->PauseNemesis delegate (atom nil)))
        test {:nodes ["n1" "n2" "n3"]}]
    (nemesis/teardown! subject test)
    (is (= [[:resume (:nodes test)] :teardown]
           @events))
    (let [failure (harness/primary-failure failure-state)]
      (is (= :nemesis (:source failure)))
      (is (= {:phase :teardown
              :component :pause
              :action :resume-processes
              :nodes (:nodes test)}
             (:context failure)))
      (is (identical? error (:throwable failure))))))

(deftest teardown-retains-each-stage-failure
  (let [failure-state (harness/failure-state)
        events (atom [])
        resume-error (RuntimeException. "resume failed")
        teardown-error (RuntimeException. "delegate teardown failed")
        delegate (failing-resume-nemesis
                  events
                  resume-error
                  teardown-error)
        subject (worker/wrap-nemesis-teardown
                 failure-state
                 :pause
                 (process/->PauseNemesis delegate (atom nil)))
        test {:nodes ["n1" "n2" "n3"]}]
    (nemesis/teardown! subject test)
    (is (= [[:resume (:nodes test)] :teardown] @events))
    (let [primary (harness/primary-failure failure-state)
          [secondary] (harness/secondary-failures failure-state)]
      (is (= :resume-processes (get-in primary [:context :action])))
      (is (identical? resume-error (:throwable primary)))
      (is (= :delegate-teardown (get-in secondary [:context :action])))
      (is (identical? teardown-error (:throwable secondary))))))

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
            (is (= [[:resume (:nodes test)]]
                   @events)))
          (finally
            (Thread/interrupted)))))))

(deftest delegate-teardown-preserves-interruptions
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
      (let [failure-state (harness/failure-state)
            events (atom [])
            delegate (failing-teardown-nemesis events error)
            subject (worker/wrap-nemesis-teardown
                     failure-state
                     :pause
                     (process/->PauseNemesis delegate (atom nil)))
            test {:nodes ["n1" "n2" "n3"]}]
        (try
          (let [[thrown interrupted?]
                (try
                  (nemesis/teardown! subject test)
                  [nil (.isInterrupted (Thread/currentThread))]
                  (catch Throwable throwable
                    [throwable (.isInterrupted (Thread/currentThread))]))]
            (is (identical? error thrown))
            (is interrupted?)
            (is (= [[:resume (:nodes test)] :teardown] @events))
            (is (nil? (harness/primary-failure failure-state))))
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
        (is (= (skipped :no-supported-leader {})
               (:value (nemesis/invoke! subject test op))))))
    (is (empty? @invocations))))

(deftest skips-known-process-precondition-misses
  (let [invocations (atom [])
        delegate (recording-nemesis invocations)
        test {:nodes ["n1"]}
        subject (process/->ProcessNemesis delegate (atom nil))]
    (testing "there is no quorum-safe kill target"
      (with-redefs [cluster/membership-status
                    (constantly {:leader "n1"})
                    cluster/voter-configs
                    (fn [_test _status] [#{"n1"}])]
        (let [value (:value
                     (nemesis/invoke! subject
                                      test
                                      {:type :info
                                       :f :kill-process
                                       :value :leader-survives}))]
          (is (= :skipped (:status value)))
          (is (= :no-quorum-safe-process-target (:reason value))))))

    (testing "there are no killed processes to restart"
      (is (= (skipped :no-processes-killed {})
             (:value (nemesis/invoke! subject
                                      test
                                      {:type :info
                                       :f :restart-process})))))

    (testing "a tracked disruption is already active"
      (let [active {:mode :leader-killed
                    :nodes ["n1"]}
            active-subject (process/->ProcessNemesis delegate
                                                     (atom active))
            value (:value
                   (nemesis/invoke! active-subject
                                    test
                                    {:type :info
                                     :f :kill-process
                                     :value :leader-killed}))]
        (is (= :skipped (:status value)))
        (is (= :processes-already-killed (:reason value)))
        (is (= active (:killed value)))))

    (is (empty? @invocations))))

(deftest requires-both-process-modes-and-an-intact-cluster
  (let [subject (#'process/coverage-checker)
        complete-history [{:f :kill-process
                           :value (installed {:mode :leader-survives})}
                          {:f :await-recovery
                           :value (installed {:leader "n1"})}
                          {:f :kill-process
                           :value (installed {:mode :leader-killed})}
                          {:f :await-recovery
                           :value (installed {:leader "n2"})}]
        missing-mode-history [{:f :kill-process
                               :value (installed {:mode :leader-survives})}
                              {:f :kill-process
                               :value (skipped
                                       :no-supported-leader
                                       {})}]
        unrecovered-history [{:f :kill-process
                              :value (installed {:mode :leader-survives})}
                             {:f :await-recovery
                              :value (installed {:leader "n1"})}
                             {:f :kill-process
                              :value (installed {:mode :leader-killed})}
                             {:f :await-recovery
                              :value (indeterminate
                                      :recovery-timeout
                                      {})}]]
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
                          :value (installed {:mode :leader-survives})}
                         {:f :await-recovery
                          :value (installed {:leader "n1"})}
                         {:f :kill-process
                          :value (installed {:mode :leader-killed})}
                         {:f :await-recovery
                          :value (installed {:leader "n2"})}]
        indeterminate-history (conj covered-history
                                    {:f :kill-process
                                     :value (indeterminate
                                             :effect-unknown
                                             {:mode :leader-killed})})
        recovered-history (conj indeterminate-history
                                {:f :await-recovery
                                 :value (installed {:leader "n2"})})]
    (let [result (checker/check subject {} indeterminate-history {})]
      (is (= :unknown (:valid? result)))
      (is (= :unknown (:cluster-state result))))
    (let [result (checker/check subject {} recovered-history {})]
      (is (:valid? result))
      (is (= :intact (:cluster-state result))))))

(deftest reanalyzes-exact-legacy-process-history
  (let [process-checker (#'process/coverage-checker)
        disruption (fn [mode nodes]
                     {:mode mode
                      :leader "n1"
                      :nodes nodes
                      :voter-configs [(set voters)]
                      :survivors (vec (remove (set nodes) voters))})
        process-history [{:f :kill-process
                          :value (disruption :leader-survives ["n2" "n3"])}
                         {:f :await-recovery :value {:leader "n1"}}
                         {:f :kill-process
                          :value (disruption :leader-killed ["n1" "n2"])}
                         {:f :await-recovery :value {:leader "n3"}}]
        pause-checker (#'process/pause-coverage-checker)
        resumed {:paused nil :resumed voters}
        pause-history [{:f :pause-process
                        :value (disruption :leader-unpaused ["n2" "n3"])}
                       {:f :resume-process :value resumed}
                       {:f :await-recovery :value {:leader "n1"}}
                       {:f :pause-process
                        :value (disruption :leader-paused ["n1" "n2"])}
                       {:f :resume-process :value resumed}
                       {:f :await-recovery :value {:leader "n3"}}]
        process-check #(checker/check process-checker {} % {})
        pause-check #(checker/check pause-checker {:nodes voters} % {})]
    (testing "exact pre-status process and pause shapes remain valid"
      (is (true? (:valid? (process-check process-history))))
      (is (true? (:valid? (pause-check pause-history)))))

    (testing "explicit disruption status overrides legacy-looking evidence"
      (doseq [status [:skipped :indeterminate]]
        (let [process-result
              (process-check (assoc-in process-history
                                       [0 :value :status]
                                       status))
              pause-result
              (pause-check (assoc-in pause-history
                                     [0 :value :status]
                                     status))]
          (is (false? (:valid? process-result)) (name status))
          (is (= [:leader-survives]
                 (:missing-modes process-result)) (name status))
          (is (false? (:valid? pause-result)) (name status))
          (is (= [:leader-unpaused]
                 (:missing-modes pause-result)) (name status)))))

    (testing "explicit recovery and resume statuses are authoritative"
      (doseq [status [:skipped :indeterminate]]
        (is (false?
             (:valid? (process-check
                       (assoc-in process-history
                                 [3 :value :status]
                                 status))))
            (name status))
        (is (not (true?
                  (:valid? (pause-check
                            (assoc-in pause-history
                                      [4 :value :status]
                                      status)))))
            (name status))))))

(deftest checks-pause-coverage-and-recovery-state
  (let [subject (#'process/pause-coverage-checker)
        test {:nodes voters}
        check #(checker/check subject test % {})
        pause-value (fn [mode nodes]
                      (installed
                       {:mode mode
                        :leader "n1"
                        :nodes nodes
                        :voter-configs [(set voters)]
                        :survivors (vec (remove (set nodes) voters))
                        :pause-results (zipmap nodes (repeat :paused))}))
        leader-unpaused (pause-value :leader-unpaused ["n2" "n3"])
        leader-paused (pause-value :leader-paused ["n1" "n2"])
        resumed-all (installed
                     {:paused nil
                      :resumed voters
                      :resume-results (zipmap voters (repeat :resumed))})
        complete-history [{:f :pause-process
                           :value leader-unpaused}
                          {:f :resume-process
                           :value resumed-all}
                          {:f :await-recovery
                           :value (installed {:leader "n1"})}
                          {:f :pause-process
                           :value leader-paused}
                          {:f :resume-process
                           :value resumed-all}
                          {:f :await-recovery
                           :value (installed {:leader "n2"})}]
        missing-mode-history [{:f :pause-process
                               :value leader-unpaused}
                              {:f :resume-process
                               :value resumed-all}
                              {:f :await-recovery
                               :value (installed {:leader "n1"})}
                              {:f :pause-process
                               :value (skipped
                                       :no-supported-leader
                                       {})}]
        unrecovered-history (-> complete-history
                                pop
                                (conj {:f :await-recovery
                                       :value (indeterminate
                                               :recovery-timeout
                                               {})}))
        indeterminate-history (conj complete-history
                                    {:f :pause-process
                                     :value (indeterminate
                                             :effect-unknown
                                             {:mode :leader-paused})})
        paused-history (conj complete-history
                             {:f :pause-process
                              :value leader-paused})
        resuming-history (conj paused-history
                               {:type :info
                                :f :resume-process})
        recovered-history (into resuming-history
                                [{:f :resume-process
                                  :value resumed-all}
                                 {:f :await-recovery
                                  :value (installed {:leader "n2"})}])
        partial-resume-history (assoc-in complete-history
                                         [4 :value :resumed]
                                         ["n1"])
        mixed-leader-pause (assoc-in leader-paused
                                     [:pause-results "n1"]
                                     :target-absent)
        mixed-pause-history (conj (subvec complete-history 0 3)
                                  {:f :pause-process
                                   :value mixed-leader-pause})
        covered-then-mixed-pause-history
        (conj complete-history
              {:f :pause-process
               :value mixed-leader-pause})
        mixed-resume-history (assoc-in complete-history
                                       [4 :value :resume-results "n5"]
                                       :target-absent)
        mixed-resume-history (assoc-in mixed-resume-history
                                       [4 :value :resume-results "n4"]
                                       :target-already-exited)
        missing-resume-result-history
        (update-in complete-history
                   [4 :value :resume-results]
                   dissoc
                   "n5")
        no-resume-history
        (assoc-in complete-history
                  [4 :value :resume-results]
                  (zipmap voters (repeat :target-absent)))]
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

    (testing "mixed pause evidence does not credit the planned leader mode"
      (is (= {:valid? false
              :missing-modes [:leader-paused]
              :cluster-state :paused}
             (select-keys (check mixed-pause-history)
                          [:valid? :missing-modes :cluster-state]))))

    (testing "a terminal mixed pause invalidates prior complete coverage"
      (is (= {:valid? false
              :missing-modes []
              :cluster-state :paused}
             (select-keys (check covered-then-mixed-pause-history)
                          [:valid? :missing-modes :cluster-state]))))

    (testing "mixed evidence confirms every target has no remaining pause"
      (is (= {:valid? true
              :cluster-state :intact}
             (select-keys (check mixed-resume-history)
                          [:valid? :cluster-state]))))

    (testing "missing resume evidence does not confirm recovery"
      (is (= {:valid? :unknown
              :cluster-state :unknown}
             (select-keys (check missing-resume-result-history)
                          [:valid? :cluster-state]))))

    (testing "an installed resume requires an actual resumed process"
      (is (= {:valid? :unknown
              :cluster-state :unknown}
             (select-keys (check no-resume-history)
                          [:valid? :cluster-state]))))

    (testing "resume completion and recovery resolve an in-flight resume"
      (is (= {:valid? true
              :cluster-state :intact}
             (select-keys (check recovered-history)
                          [:valid? :cluster-state]))))))
