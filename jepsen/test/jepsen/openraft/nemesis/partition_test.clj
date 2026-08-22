(ns jepsen.openraft.nemesis.partition-test
  (:require [clojure.set :as set]
            [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
             [control :as c]
             [nemesis :as nemesis]]
            [jepsen.openraft [cluster :as cluster]
             [harness :as harness]
             [worker :as worker]]
            [jepsen.openraft.nemesis.partition :as partition]
            [jepsen.openraft.quorum :as quorum]))

(def nodes ["n1" "n2" "n3"])

(defn- installed [details]
  (assoc details :status :installed))

(defn- skipped [reason details]
  (assoc details :status :skipped :reason reason))

(defn- indeterminate [reason details]
  (assoc details :status :indeterminate :reason reason))

(defn- teardown-partitioner [teardown]
  (reify nemesis/Nemesis
    (setup! [this _test]
      this)
    (invoke! [_ _test op]
      op)
    (teardown! [_ _test]
      (teardown))))

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

(deftest starts-partition-only-with-a-supported-leader
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
    (testing "a quorum-supported leader determines the partition"
      (with-redefs [cluster/membership-status
                    (fn [_test]
                      {:leader "n2"})
                    cluster/voter-configs
                    (fn [_test _status]
                      [(set nodes)])]
        (let [result (nemesis/invoke! subject {:nodes nodes} op)
              delegated (first @invocations)]
          (is (= :start (:f delegated)))
          (is (= #{"n2"}
                 (-> result :value :components first set)))
          (is (= :leader-in-minority
                 (-> result :value :mode)))
          (is (= "n2"
                 (-> result :value :leader)))
          (is (= :installed (get-in result [:value :status]))))))

    (testing "no partition is installed without a supported leader"
      (reset! invocations [])
      (with-redefs [cluster/membership-status (constantly nil)]
        (is (= (skipped :no-supported-leader {})
               (:value (nemesis/invoke! subject {:nodes nodes} op))))
        (is (empty? @invocations))))

    (testing "no partition is installed without a safe target"
      (reset! invocations [])
      (with-redefs [cluster/membership-status
                    (constantly {:leader "n1"})
                    cluster/voter-configs
                    (fn [_test _status] [#{"n1"}])]
        (let [value (:value (nemesis/invoke! subject
                                             {:nodes ["n1"]}
                                             (assoc op
                                                    :value
                                                    :leader-in-majority)))]
          (is (= :skipped (:status value)))
          (is (= :no-safe-partition-target (:reason value)))
          (is (empty? @invocations)))))))

(deftest partition-cleanup-failure-records-a-harness-failure
  (let [failure-state (harness/failure-state)
        error (ex-info "unreachable" {:kind :unreachable})
        attempted? (atom false)
        partitioner (teardown-partitioner
                     #(do
                        (reset! attempted? true)
                        (throw error)))
        subject (worker/wrap-nemesis-teardown
                 failure-state
                 :partition
                 (partition/->PartitionNemesis partitioner))]
    (nemesis/teardown! subject
                       {:nodes nodes})
    (is @attempted?)
    (let [failure (harness/primary-failure failure-state)]
      (is (= :nemesis (:source failure)))
      (is (= {:phase :teardown
              :component :partition
              :nodes nodes}
             (:context failure)))
      (is (identical? error (:throwable failure))))))

(deftest partition-control-failures-take-the-harness-path
  (let [error (ex-info "SSH failed" {:kind :ssh})
        partitioner (reify nemesis/Nemesis
                      (setup! [this _test]
                        this)
                      (invoke! [_ _test _op]
                        (throw error))
                      (teardown! [this _test]
                        this))
        failure-state (harness/failure-state)
        subject (worker/wrap-nemesis
                 failure-state
                 (partition/->PartitionNemesis partitioner))
        setup-subject (nemesis/setup! subject {:nodes nodes})
        op {:type :info
            :process :nemesis
            :f :start-partition
            :value :leader-in-minority}
        thrown (with-redefs [cluster/membership-status
                             (constantly {:leader "n1"})
                             cluster/voter-configs
                             (fn [_test _status] [(set nodes)])]
                 (try
                   (nemesis/invoke! setup-subject {:nodes nodes} op)
                   nil
                   (catch Exception e
                     e)))]
    (is (identical? error thrown))
    (is (identical? error
                    (:throwable (harness/primary-failure failure-state))))))

(deftest runtime-partition-control-restores-receiving-thread-interruption
  (let [test {:nodes nodes
              :sessions (zipmap nodes (repeat :unused-session))}
        cases [[:interrupted-exception
                #(InterruptedException. "interrupted")]
               [:interrupted-io
                #(java.io.InterruptedIOException. "interrupted")]
               [:closed-by-interrupt
                #(java.nio.channels.ClosedByInterruptException.)]
               [:wrapped
                #(ex-info "interrupted" {:kind :interrupted})]]
        operations [[:start
                     {:type :info
                      :process :nemesis
                      :f :start-partition
                      :value :leader-in-minority}]
                    [:stop
                     {:type :info
                      :process :nemesis
                      :f :stop-partition}]]]
    (doseq [[operation op] operations
            [interruption make-error] cases]
      (testing (str (name operation) " " (name interruption))
        (Thread/interrupted)
        (try
          (let [error (make-error)
                partitioner
                (reify nemesis/Nemesis
                  (setup! [this _test]
                    this)
                  (invoke! [_ test _op]
                    (c/on-nodes test
                                (fn [_test node]
                                  (if (= "n2" node)
                                    (throw error)
                                    :ok))))
                  (teardown! [this _test]
                    this))
                failure-state (harness/failure-state)
                subject (->> partitioner
                             partition/->PartitionNemesis
                             (worker/wrap-nemesis failure-state))
                setup-subject (nemesis/setup! subject test)
                [thrown interrupted?]
                (with-redefs [cluster/membership-status
                              (constantly {:leader "n1"})
                              cluster/voter-configs
                              (fn [_test _status] [(set nodes)])]
                  (try
                    (nemesis/invoke! setup-subject test op)
                    [nil (.isInterrupted (Thread/currentThread))]
                    (catch Exception e
                      [e (.isInterrupted (Thread/currentThread))])))]
            (is (identical? error thrown))
            (is interrupted?)
            (is (nil? (harness/primary-failure failure-state))))
          (finally
            (Thread/interrupted)))))))

(deftest partition-cleanup-preserves-interruptions
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
      (let [partitioner (teardown-partitioner #(throw error))
            subject (partition/->PartitionNemesis partitioner)]
        (try
          (let [thrown (try
                         (nemesis/teardown! subject {:nodes nodes})
                         nil
                         (catch Exception e
                           e))
                interrupted? (.isInterrupted (Thread/currentThread))]
            (is (identical? error thrown))
            (is interrupted?))
          (finally
            (Thread/interrupted)))))))

(deftest requires-both-partitions-and-an-intact-cluster
  (let [subject (#'partition/coverage-checker)
        complete-history [{:f :start-partition
                           :value (installed {:mode :leader-in-majority})}
                          {:f :stop-partition
                           :value (installed {})}
                          {:f :start-partition
                           :value (installed {:mode :leader-in-minority})}
                          {:f :stop-partition
                           :value (installed {})}
                          {:f :await-recovery
                           :value (installed {:leader "n2"})}]
        missing-mode-history [{:f :start-partition
                               :value (installed
                                       {:mode :leader-in-majority})}
                              {:f :stop-partition
                               :value (installed {})}
                              {:f :start-partition
                               :value (skipped :no-supported-leader {})}
                              {:f :stop-partition
                               :value (installed {})}
                              {:f :await-recovery
                               :value (installed {:leader "n2"})}]
        unrecovered-history [{:f :start-partition
                              :value (installed
                                      {:mode :leader-in-majority})}
                             {:f :stop-partition
                              :value (installed {})}
                             {:f :start-partition
                              :value (installed
                                      {:mode :leader-in-minority})}
                             {:f :stop-partition
                              :value (installed {})}
                             {:f :await-recovery
                              :value (indeterminate
                                      :recovery-timeout
                                      {})}]]
    (testing "both modes complete and the cluster recovers"
      (let [result (checker/check subject {} complete-history {})]
        (is (:valid? result))
        (is (= :intact (:cluster-state result)))))
    (testing "a skipped partition does not count toward coverage"
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
                  :value (installed {:mode :leader-in-majority
                                     :leader "n1"})}
                 {:type :info
                  :process :nemesis
                  :f :stop-partition
                  :value nil}
                 {:type :info
                  :process :nemesis
                  :f :stop-partition
                  :value (installed {})}
                 {:type :info
                  :process :nemesis
                  :f :start-partition
                  :value :leader-in-minority}
                 {:type :info
                  :process :nemesis
                  :f :start-partition
                  :value (installed {:mode :leader-in-minority
                                     :leader "n1"})}
                 {:type :info
                  :process :nemesis
                  :f :stop-partition
                  :value nil}
                 {:type :info
                  :process :nemesis
                  :f :stop-partition
                  :value (installed {})}
                 {:type :info
                  :process :nemesis
                  :f :await-recovery
                  :value nil}
                 {:type :info
                  :process :nemesis
                  :f :await-recovery
                  :value (installed {:leader "n2"})}
                 {:type :info
                  :process :nemesis
                  :f :await-recovery
                  :value nil}
                 {:type :info
                  :process :nemesis
                  :f :await-recovery
                  :value (installed {:leader "n2"})}]
        result (checker/check subject {} history {})]
    (is (:valid? result))
    (is (= [:leader-in-majority :leader-in-minority]
           (:observed-modes result)))
    (is (= :intact (:cluster-state result)))))

(deftest reports-an-indeterminate-partition-state
  (let [subject (#'partition/coverage-checker)
        covered-history [{:f :start-partition
                          :value (installed {:mode :leader-in-majority})}
                         {:f :stop-partition
                          :value (installed {})}
                         {:f :start-partition
                          :value (installed {:mode :leader-in-minority})}
                         {:f :stop-partition
                          :value (installed {})}
                         {:f :await-recovery
                          :value (installed {:leader "n2"})}]
        indeterminate-history
        (conj covered-history
              {:type :info
               :process :nemesis
               :f :start-partition
               :value (indeterminate :effect-unknown
                                     {:mode :leader-in-majority})})
        result (checker/check subject {} indeterminate-history {})]
    (is (= :unknown (:valid? result)))
    (is (= :unknown (:cluster-state result)))))

(deftest reports-an-indeterminate-heal-state
  (let [subject (#'partition/coverage-checker)
        history-before-heal [{:f :start-partition
                              :value (installed
                                      {:mode :leader-in-majority})}
                             {:f :stop-partition
                              :value (installed {})}
                             {:f :start-partition
                              :value (installed
                                      {:mode :leader-in-minority})}
                             {:f :stop-partition
                              :value (indeterminate
                                      :effect-unknown
                                      {})}]
        indeterminate-history (conj history-before-heal
                                    {:f :await-recovery
                                     :value (installed {:leader "n2"})})
        recovered-history (into history-before-heal
                                [{:f :stop-partition
                                  :value (installed {})}
                                 {:f :await-recovery
                                  :value (installed {:leader "n2"})}])]
    (testing "a successful readiness check cannot prove that a failed heal completed"
      (let [result (checker/check subject {} indeterminate-history {})]
        (is (= :unknown (:valid? result)))
        (is (= :unknown (:cluster-state result)))))
    (testing "a later confirmed heal and recovery restore the intact state"
      (let [result (checker/check subject {} recovered-history {})]
        (is (:valid? result))
        (is (= :intact (:cluster-state result)))))))
